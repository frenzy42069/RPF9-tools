use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use crate::{
    attr_dir, attr_file,
    cache::{Cache, Namespace},
    range::{self, StreamRecord},
    readable::{self, Dir, FileKind, Options, Projection},
    ROOT,
};
use fuser::{
    Errno, FileHandle, FileType, Filesystem, Generation, INodeNo, ReplyAttr, ReplyData,
    ReplyDirectory, ReplyEntry, ReplyOpen,
};

const RANGE: u64 = 64 * 1024 * 1024;
#[derive(Clone)]
enum Source {
    Directory(PathBuf),
    Archive(PathBuf, u64),
    Range(PathBuf, u64, u64),
    Physical(PathBuf, u64),
    Stream(PathBuf, StreamRecord),
    ReadDir(PathBuf, Dir),
    ReadFile(PathBuf, FileKind),
}
struct Node {
    name: OsString,
    parent: INodeNo,
    source: Source,
    children: Option<Vec<INodeNo>>,
}
struct State {
    nodes: HashMap<INodeNo, Node>,
    next: u64,
}
pub struct Tree {
    state: Mutex<State>,
    cache: Cache,
    readable: Option<Options>,
    handles: Mutex<HashMap<u64, Arc<File>>>,
    excluded: Vec<PathBuf>,
    recover_names: bool,
    next_handle: std::sync::atomic::AtomicU64,
}

impl Tree {
    pub fn new(root: PathBuf, cache: Cache, readable: Option<Options>) -> Self {
        let mut nodes = HashMap::new();
        nodes.insert(
            ROOT,
            Node {
                name: OsString::new(),
                parent: ROOT,
                source: if root.is_dir() {
                    Source::Directory(root)
                } else if readable.is_some()
                    && root
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("ttpf"))
                {
                    Source::ReadDir(root, Dir::Pack)
                } else {
                    let size = root.metadata().map(|m| m.len()).unwrap_or(0);
                    Source::Archive(root, size)
                },
                children: None,
            },
        );
        Self {
            state: Mutex::new(State { nodes, next: 2 }),
            cache,
            readable,
            handles: Mutex::new(HashMap::new()),
            excluded: Vec::new(),
            recover_names: true,
            next_handle: std::sync::atomic::AtomicU64::new(1 << 63),
        }
    }
    pub fn with_paths(mut self, mountpoint: PathBuf, cache: PathBuf, recover_names: bool) -> Self {
        self.excluded = vec![mountpoint, cache];
        self.recover_names = recover_names;
        self
    }

    fn listing(&self, source: &Source) -> io::Result<Vec<(OsString, Source)>> {
        match source {
            Source::Directory(path) => {
                let mut result = Vec::new();
                for item in std::fs::read_dir(path)? {
                    let item = item?;
                    let meta = item.file_type()?;
                    let p = item.path();
                    if self.excluded.contains(&p) {
                        continue;
                    }
                    let source = if meta.is_dir() {
                        Source::Directory(p)
                    } else if meta.is_file() {
                        let n = item.metadata()?.len();
                        if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("rpf")) {
                            Source::Archive(p, n)
                        } else if self.readable.is_some()
                            && p.extension()
                                .is_some_and(|e| e.eq_ignore_ascii_case("ttpf"))
                        {
                            Source::ReadDir(p, Dir::Pack)
                        } else {
                            Source::Physical(p, n)
                        }
                    } else {
                        continue;
                    };
                    result.push((item.file_name(), source));
                }
                result.sort_by(|a, b| a.0.cmp(&b.0));
                Ok(result)
            }
            Source::Archive(path, size) => {
                if *size <= RANGE {
                    return self.listing(&Source::Range(path.clone(), 0, *size));
                }
                Ok((0..*size)
                    .step_by(RANGE as usize)
                    .map(|n| {
                        (
                            format!("range-{n:010x}").into(),
                            Source::Range(path.clone(), n, (*size - n).min(RANGE)),
                        )
                    })
                    .collect())
            }
            Source::Range(path, start, len) => {
                let selector = format!("range-v1-{start:x}-{len:x}");
                let file =
                    self.cache
                        .get_or_create(Namespace::Index, path, &selector, |destination| {
                            let entries = range::index(path, *start, *len)?;
                            serde_json::to_writer(File::create(destination)?, &entries)
                                .map_err(io::Error::other)
                        })?;
                let entries: Vec<StreamRecord> =
                    serde_json::from_reader(file).map_err(io::Error::other)?;
                let mut result = Vec::new();
                let mut names = rpf9_core::names::Filenames::default();
                for r in entries {
                    let stem = if self.recover_names {
                        r.stem.as_deref()
                    } else {
                        None
                    };
                    let n = names.for_entry(r.offset as usize, &r.ext, stem);
                    if let Some(options) = &self.readable {
                        if let Some((name, projection)) = readable::project(path, &r, options) {
                            let source = match projection {
                                Projection::Dir(dir) => Source::ReadDir(path.clone(), dir),
                                Projection::File(kind) => Source::ReadFile(path.clone(), kind),
                            };
                            let name = if let Some((stem, ext)) = name.rsplit_once('.') {
                                names.for_entry(r.offset as usize, ext, Some(stem))
                            } else {
                                names.for_entry(r.offset as usize, "asset", Some(&name))
                            };
                            result.push((name.into(), source));
                        }
                    }
                    result.push((n.into(), Source::Stream(path.clone(), r)));
                }
                Ok(result)
            }
            Source::ReadDir(path, dir) => {
                let options = self.readable.as_ref().ok_or(io::ErrorKind::NotFound)?;
                let selector = format!("readable-list-v2-{dir:?}-{options:?}");
                let file =
                    self.cache
                        .get_or_create(Namespace::Index, path, &selector, |destination| {
                            let list = readable::list(path, dir, options)?;
                            serde_json::to_writer(File::create(destination)?, &list)
                                .map_err(io::Error::other)
                        })?;
                let list: Vec<(String, FileKind)> =
                    serde_json::from_reader(file).map_err(io::Error::other)?;
                let mut names = std::collections::HashSet::new();
                let mut result = Vec::new();
                for (index, (name, kind)) in list.into_iter().enumerate() {
                    let mut candidate = name.clone();
                    if !names.insert(candidate.to_ascii_lowercase()) {
                        let (stem, ext) = name.rsplit_once('.').unwrap_or((&name, ""));
                        let mut suffix = index;
                        loop {
                            candidate = if ext.is_empty() {
                                format!("{stem}-{suffix}")
                            } else {
                                format!("{stem}-{suffix}.{ext}")
                            };
                            if names.insert(candidate.to_ascii_lowercase()) {
                                break;
                            }
                            suffix += 1;
                        }
                    }
                    result.push((candidate.into(), Source::ReadFile(path.clone(), kind)));
                }
                if matches!(dir, Dir::Pack) {
                    result.push((
                        "original.ttpf".into(),
                        Source::Physical(path.clone(), path.metadata()?.len()),
                    ));
                }
                Ok(result)
            }
            _ => Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "not a directory",
            )),
        }
    }
    fn children(&self, ino: INodeNo) -> io::Result<Vec<INodeNo>> {
        let source = {
            let state = self.state.lock().unwrap();
            let node = state.nodes.get(&ino).ok_or(io::ErrorKind::NotFound)?;
            if let Some(c) = &node.children {
                return Ok(c.clone());
            }
            node.source.clone()
        };
        let entries = self.listing(&source)?;
        let mut state = self.state.lock().unwrap();
        if let Some(c) = &state
            .nodes
            .get(&ino)
            .ok_or(io::ErrorKind::NotFound)?
            .children
        {
            return Ok(c.clone());
        }
        let mut result = Vec::new();
        for (name, source) in entries {
            let id = INodeNo(state.next);
            state.next += 1;
            state.nodes.insert(
                id,
                Node {
                    name,
                    parent: ino,
                    source,
                    children: None,
                },
            );
            result.push(id);
        }
        state.nodes.get_mut(&ino).unwrap().children = Some(result.clone());
        Ok(result)
    }
    fn source(&self, ino: INodeNo) -> Option<Source> {
        self.state
            .lock()
            .ok()?
            .nodes
            .get(&ino)
            .map(|n| n.source.clone())
    }
    fn attr(&self, ino: INodeNo) -> Option<fuser::FileAttr> {
        let source = self.source(ino)?;
        Some(match source {
            Source::Physical(_, n) => attr_file(ino, n),
            Source::Stream(_, ref r) => attr_file(ino, if r.raw { r.stored } else { r.size }),
            Source::ReadFile(ref path, ref kind) => {
                let options = self.readable.as_ref()?;
                let key = readable::key(kind, options);
                let size = self
                    .cache
                    .get(Namespace::Media, path, &key)
                    .ok()
                    .flatten()
                    .and_then(|f| f.metadata().ok())
                    .map_or_else(|| kind.listed_size(), |m| m.len());
                attr_file(ino, size)
            }
            _ => attr_dir(ino),
        })
    }
}
impl Filesystem for Tree {
    fn lookup(&self, _: &fuser::Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        match self.children(parent) {
            Ok(children) => {
                let state = self.state.lock().unwrap();
                let found = children.into_iter().find(|i| state.nodes[i].name == name);
                drop(state);
                if let Some(i) = found {
                    if let Some(attr) = self.attr(i) {
                        reply.entry(&std::time::Duration::ZERO, &attr, Generation(0));
                        return;
                    }
                }
                reply.error(Errno::ENOENT)
            }
            Err(_) => reply.error(Errno::EIO),
        }
    }
    fn getattr(
        &self,
        _: &fuser::Request,
        ino: INodeNo,
        handle: Option<FileHandle>,
        reply: ReplyAttr,
    ) {
        let attr = handle
            .and_then(|handle| {
                let file = self.handles.lock().unwrap().get(&handle.0).cloned()?;
                Some(attr_file(ino, file.metadata().ok()?.len()))
            })
            .or_else(|| self.attr(ino));
        match attr {
            Some(a) => reply.attr(&std::time::Duration::ZERO, &a),
            None => reply.error(Errno::ENOENT),
        }
    }
    fn readdir(
        &self,
        _: &fuser::Request,
        ino: INodeNo,
        _: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        match self.children(ino) {
            Ok(children) => {
                let state = self.state.lock().unwrap();
                if offset == 0 && reply.add(ino, 1, FileType::Directory, ".") {
                    reply.ok();
                    return;
                }
                if offset <= 1 && reply.add(state.nodes[&ino].parent, 2, FileType::Directory, "..")
                {
                    reply.ok();
                    return;
                }
                for (i, id) in children
                    .iter()
                    .enumerate()
                    .skip(offset.saturating_sub(2) as usize)
                {
                    let n = &state.nodes[id];
                    let kind = match n.source {
                        Source::Physical(..) | Source::Stream(..) | Source::ReadFile(..) => {
                            FileType::RegularFile
                        }
                        _ => FileType::Directory,
                    };
                    if reply.add(*id, (i + 3) as u64, kind, &n.name) {
                        break;
                    }
                }
                reply.ok()
            }
            Err(_) => reply.error(Errno::EIO),
        }
    }
    fn open(
        &self,
        _: &fuser::Request,
        ino: INodeNo,
        flags: fuser::OpenFlags,
        _: bool,
        reply: ReplyOpen,
    ) {
        if flags.acc_mode() != fuser::OpenAccMode::O_RDONLY {
            reply.error(Errno::EROFS);
            return;
        }
        if let Some(Source::ReadFile(path, kind)) = self.source(ino) {
            let Some(options) = &self.readable else {
                reply.error(Errno::EIO);
                return;
            };
            let key = readable::key(&kind, options);
            match self
                .cache
                .get_or_create(Namespace::Media, &path, &key, |destination| {
                    readable::generate(&path, &kind, options, destination)
                }) {
                Ok(file) => {
                    let handle = self
                        .next_handle
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    self.handles.lock().unwrap().insert(handle, Arc::new(file));
                    reply.opened(FileHandle(handle), fuser::FopenFlags::FOPEN_DIRECT_IO);
                }
                Err(error) => {
                    eprintln!("rpf9-fuse: preview {}: {error}", path.display());
                    reply.error(Errno::EIO);
                }
            }
            return;
        }
        match self.source(ino) {
            Some(Source::Physical(..) | Source::Stream(..)) => {
                reply.opened(FileHandle(0), fuser::FopenFlags::empty())
            }
            Some(_) => reply.error(Errno::EISDIR),
            None => reply.error(Errno::ENOENT),
        }
    }
    fn read(
        &self,
        _: &fuser::Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        _: fuser::OpenFlags,
        _: Option<fuser::LockOwner>,
        reply: ReplyData,
    ) {
        let result = (|| -> io::Result<Vec<u8>> {
            match self.source(ino).ok_or(io::ErrorKind::NotFound)? {
                Source::Physical(path, n) => read_range(&path, 0, n, offset, size),
                Source::ReadFile(_, _) => {
                    let file = self
                        .handles
                        .lock()
                        .unwrap()
                        .get(&fh.0)
                        .cloned()
                        .ok_or(io::ErrorKind::NotFound)?;
                    let available = file
                        .metadata()?
                        .len()
                        .saturating_sub(offset)
                        .min(size as u64) as usize;
                    let mut data = vec![0; available];
                    file.read_exact_at(&mut data, offset)?;
                    Ok(data)
                }
                Source::Stream(path, r) if r.raw => {
                    read_range(&path, r.offset, r.stored, offset, size)
                }
                Source::Stream(path, r) => {
                    let mut file = File::open(path)?;
                    file.seek(SeekFrom::Start(r.offset))?;
                    let mut compressed =
                        vec![0; usize::try_from(r.stored).map_err(|_| io::ErrorKind::InvalidData)?];
                    file.read_exact(&mut compressed)?;
                    let mut decoded = Vec::new();
                    rpf9_core::stream::inflate(&compressed, 0, &mut decoded)
                        .ok_or(io::ErrorKind::InvalidData)?;
                    let a = (offset as usize).min(decoded.len());
                    let b = a.saturating_add(size as usize).min(decoded.len());
                    Ok(decoded[a..b].to_vec())
                }
                _ => Err(io::ErrorKind::IsADirectory.into()),
            }
        })();
        match result {
            Ok(b) => reply.data(&b),
            Err(_) => reply.error(Errno::EIO),
        }
    }
    fn release(
        &self,
        _: &fuser::Request,
        _: INodeNo,
        fh: FileHandle,
        _: fuser::OpenFlags,
        _: Option<fuser::LockOwner>,
        _: bool,
        reply: fuser::ReplyEmpty,
    ) {
        self.handles.lock().unwrap().remove(&fh.0);
        reply.ok();
    }
}
fn read_range(path: &Path, base: u64, len: u64, off: u64, size: u32) -> io::Result<Vec<u8>> {
    let n = len.saturating_sub(off).min(size as u64) as usize;
    let mut data = vec![0; n];
    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(
        base.checked_add(off).ok_or(io::ErrorKind::InvalidInput)?,
    ))?;
    f.read_exact(&mut data)?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lazy_tree_reads_directory_and_reuses_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let source = dir.path().join("small.rpf");
        let mut data = vec![0u8; 128];
        data[..4].copy_from_slice(b"9FPR");
        std::fs::write(&source, data).unwrap();
        let tree = Tree::new(
            dir.path().to_path_buf(),
            Cache::new(cache.path().to_path_buf(), 100_000).unwrap(),
            None,
        );
        let root = tree.children(ROOT).unwrap();
        assert_eq!(root.len(), 1);
        let listed = tree.children(root[0]).unwrap();

        assert_eq!(listed, tree.children(root[0]).unwrap());
    }
    fn child(tree: &Tree, parent: INodeNo, name: &str) -> INodeNo {
        let children = tree.children(parent).unwrap();
        let state = tree.state.lock().unwrap();
        *children
            .iter()
            .find(|inode| state.nodes[inode].name == name)
            .unwrap_or_else(|| panic!("missing {name} under {parent:?}"))
    }

    fn materialize(tree: &Tree, inode: INodeNo) -> File {
        let Source::ReadFile(path, kind) = tree.source(inode).unwrap() else {
            panic!("expected a virtual readable file");
        };
        let options = tree.readable.as_ref().unwrap();
        let key = readable::key(&kind, options);
        tree.cache
            .get_or_create(Namespace::Media, &path, &key, |destination| {
                readable::generate(&path, &kind, options, destination)
            })
            .unwrap()
    }

    #[test]
    fn video_preview_reports_nonzero_size_before_conversion() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("movies.rpf");
        let mut data = vec![0; 64];
        data[..4].copy_from_slice(b"9FPR");
        data[16..20].copy_from_slice(b"KB2n");
        data[20..24].copy_from_slice(&16_u32.to_le_bytes());
        data[24..28].copy_from_slice(&1_u32.to_le_bytes());
        std::fs::write(&source, data).unwrap();

        let ffmpeg = dir.path().join("ffmpeg");
        std::fs::write(
            &ffmpeg,
            "#!/bin/sh\nfor output do :; done\nprintf fake-mp4 > \"$output\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&ffmpeg, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cache = dir.path().join("cache");
        let tree = Tree::new(
            source,
            Cache::new(cache.clone(), 100_000).unwrap(),
            Some(Options {
                ffmpeg: Some(ffmpeg),
                ..Options::default()
            }),
        );

        let preview = child(&tree, ROOT, "00000010.mp4");
        assert_eq!(tree.attr(preview).unwrap().size, 24);
        assert_eq!(cache.join("media").read_dir().unwrap().count(), 0);
        let file = materialize(&tree, preview);
        assert_eq!(file.metadata().unwrap().len(), 8);
        assert_eq!(tree.attr(preview).unwrap().size, 8);
    }

    #[test]
    fn build_archive_views_are_lazy_and_generate_media() {
        let Some(source) = std::env::var_os("RPF9_TEST_BUILD").map(PathBuf::from) else {
            return;
        };
        let cache = tempfile::tempdir().unwrap();
        let options = Options {
            audio_extractor: std::env::var_os("RPF9_TEST_AUDIO_EXTRACTOR").map(PathBuf::from),
            ffmpeg: None,
        };
        let audio_available = options.audio_extractor.is_some();
        let tree = Tree::new(
            source,
            Cache::new(cache.path().to_path_buf(), 100_000_000).unwrap(),
            Some(options),
        );
        let levels = child(&tree, ROOT, "levels.rpf");
        let range = child(&tree, levels, "range-00f8000000");
        assert_eq!(cache.path().join("index").read_dir().unwrap().count(), 0);
        let model = child(&tree, range, "int_abd_1_gfa_rt_1_shl.glb");
        assert_eq!(cache.path().join("index").read_dir().unwrap().count(), 1);
        assert_eq!(cache.path().join("media").read_dir().unwrap().count(), 0);
        let mut magic = [0; 4];
        materialize(&tree, model).read_exact(&mut magic).unwrap();
        assert_eq!(&magic, b"glTF");
        let textures = child(&tree, ROOT, "textures.rpf");
        let range = child(&tree, textures, "range-0000000000");
        let dictionary = child(&tree, range, "00002010.textures");
        let image = child(&tree, dictionary, "pistol_glass_crack_decals-000000.png");
        materialize(&tree, image).read_exact(&mut magic).unwrap();
        assert_eq!(&magic, b"\x89PNG");
        if audio_available {
            let audio = child(&tree, ROOT, "audio.rpf");
            let range = child(&tree, audio, "range-0000000000");
            let bank = child(&tree, range, "013da000.tracks");
            let track = child(&tree, bank, "00007-55.opus");
            let size = tree.attr(track).unwrap().size;
            assert!(size > 0);
            let mut audio = materialize(&tree, track);
            assert_eq!(audio.metadata().unwrap().len(), size);
            audio.read_exact(&mut magic).unwrap();
            assert_eq!(&magic, b"OggS");
        }
    }
    #[test]
    fn texture_pack_is_a_folder_of_seekable_images() {
        let dir = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let mut bytes = vec![0_u8; 0x10200];
        bytes[..4].copy_from_slice(b"RDS5");
        bytes[0x100..0x104].copy_from_slice(b"ETPF");
        for (offset, value) in [
            (4, 3_u32),
            (16, 1),
            (24, 2),
            (32, 258),
            (36, 4),
            (0x108, 1),
            (0x110, 255 | (255 << 14)),
            (0x114, 74 << 3 | 0x0100_0000),
            (0x118, 1),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        std::fs::write(dir.path().join("textures.ttpf"), &bytes).unwrap();
        let tree = Tree::new(
            dir.path().to_path_buf(),
            Cache::new(cache.path().to_path_buf(), 100_000).unwrap(),
            Some(Options::default()),
        );
        let textures = child(&tree, ROOT, "textures.ttpf");
        child(&tree, textures, "original.ttpf");
        let image = child(&tree, textures, "000000.png");
        assert_eq!(cache.path().join("media").read_dir().unwrap().count(), 0);
        let mut file = materialize(&tree, image);
        let mut magic = [0; 4];
        file.read_exact(&mut magic).unwrap();
        assert_eq!(&magic, b"\x89PNG");
        file.seek(SeekFrom::End(-4)).unwrap();
        file.read_exact(&mut magic).unwrap();
    }
}
