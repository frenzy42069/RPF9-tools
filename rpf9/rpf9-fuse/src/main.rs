//! `rpf9-fuse` — mount RPF9 archives as folders (fuser/FUSE).
//!
//! Decryption-less: entries are resources recovered from zlib streams and
//! carved media; names are offset-based (`h<archive:06x>/<at:08x>.<ext>`)
//! until the RPF9 native directory is decoded.
//!
//! Usage: rpf9-fuse <rpf> [mntpoint]

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use fuser::{
    Config, Errno, FileAttr, FileHandle, FileType, Filesystem, Generation, INodeNo, ReplyAttr,
    ReplyData, ReplyDirectory, ReplyEntry, ReplyOpen,
};
use memmap2::Mmap;
use rpf9_core::archive::Archive;
use rpf9_core::kinds::Kind;
use rpf9_core::scan::Scan;
use rpf9_core::stream::Stream;

const ROOT: INodeNo = INodeNo(1);
const TTL: Duration = Duration::from_secs(3600);

struct RpfFile {
    name: String,
    mmap: Mmap,
    archives: Vec<Archive>,
    streams: Vec<Vec<Stream>>, // streams[a] — streams of archives[a]
    dir_inos: Vec<INodeNo>,    // dir_inos[a] — inode of archive a's directory
}

struct Entry {
    name: OsString,
    kind: FileType,
    stream: Option<(usize, usize, usize)>, // (file, archive, stream)
}

struct RpfFs {
    files: Vec<RpfFile>,
    entries: HashMap<INodeNo, Entry>,
    children: HashMap<INodeNo, Vec<INodeNo>>,
}

fn attr_dir(ino: INodeNo) -> FileAttr {
    FileAttr {
        ino,
        size: 0,
        blocks: 0,
        atime: SystemTime::now(),
        mtime: SystemTime::now(),
        ctime: SystemTime::now(),
        crtime: SystemTime::now(),
        kind: FileType::Directory,
        perm: 0o755,
        nlink: 2,
        uid: 0,
        gid: 0,
        rdev: 0,
        blksize: 4096,
        flags: 0,
    }
}

fn attr_file(ino: INodeNo, size: u64) -> FileAttr {
    FileAttr {
        size,
        blocks: size.div_ceil(512),
        atime: SystemTime::now(),
        mtime: SystemTime::now(),
        ctime: SystemTime::now(),
        crtime: SystemTime::now(),
        kind: FileType::RegularFile,
        perm: 0o644,
        nlink: 1,
        ..attr_dir(ino)
    }
}

impl RpfFs {
    fn attr_of(&self, ino: INodeNo) -> Option<FileAttr> {
        if ino == ROOT {
            return Some(attr_dir(ino));
        }
        let e = self.entries.get(&ino)?;
        if e.kind == FileType::RegularFile {
            let (f, a, j) = e.stream.expect("regular file entry");
            let s = &self.files[f].streams[a][j];
            let size = if s.kind == Kind::Movie { s.stored_len } else { s.decoded_len };
            return Some(attr_file(ino, size as u64));
        }
        Some(attr_dir(ino))
    }

    /// Decode the stream at `(f, a, j)` into memory.
    fn read_stream(&self, f: usize, a: usize, j: usize) -> io::Result<Vec<u8>> {
        let s = &self.files[f].streams[a][j];
        let data = &self.files[f].mmap;
        match s.kind {
            Kind::Movie | Kind::Adat | Kind::Png | Kind::Riff { .. } => {
                Ok(data[s.at..s.at + s.stored_len].to_vec())
            }
            _ => {
                let mut out = Vec::new();
                if rpf9_core::stream::inflate(data, s.at, &mut out).is_none() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("inflate failed at {:#x}", s.at),
                    ));
                }
                Ok(out)
            }
        }
    }
}

impl Filesystem for RpfFs {
    fn lookup(
        &self,
        _req: &fuser::Request,
        parent: INodeNo,
        name: &std::ffi::OsStr,
        reply: ReplyEntry,
    ) {
        if let Some(children) = self.children.get(&parent) {
            if let Some(&ino) = children.iter().find(|&&ino| self.entries[&ino].name == name) {
                match self.attr_of(ino) {
                    Some(attr) => reply.entry(&TTL, &attr, Generation(0)),
                    None => reply.error(Errno::ENOENT),
                }
                return;
            }
        }
        reply.error(Errno::ENOENT);
    }

    fn getattr(&self, _req: &fuser::Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        match self.attr_of(ino) {
            Some(attr) => reply.attr(&TTL, &attr),
            None => reply.error(Errno::ENOENT),
        }
    }

    fn readdir(
        &self,
        _req: &fuser::Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        if let Some(children) = self.children.get(&ino) {
            for (i, &child) in children.iter().enumerate() {
                let entry_offset = (i + 1) as u64;
                if entry_offset <= offset {
                    continue;
                }
                if reply.remaining_capacity() == 0 {
                    break;
                }
                let e = &self.entries[&child];
                if reply.add(child, entry_offset, e.kind, &e.name) {
                    break;
                }
            }
        }
        reply.ok();
    }

    fn open(&self, _req: &fuser::Request, ino: INodeNo, _flags: fuser::OpenFlags, _kill: bool, reply: ReplyOpen) {
        reply.opened(FileHandle(ino.0), fuser::FopenFlags::empty());
    }

    fn read(
        &self,
        _req: &fuser::Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: fuser::OpenFlags,
        _lock_owner: Option<fuser::LockOwner>,
        reply: ReplyData,
    ) {
        let Some(e) = self.entries.get(&ino) else {
            reply.error(Errno::ENOENT);
            return;
        };
        let Some((f, a, j)) = e.stream else {
            reply.error(Errno::EISDIR);
            return;
        };
        match self.read_stream(f, a, j) {
            Ok(buf) => {
                let start = (offset as usize).min(buf.len());
                let end = ((offset + size as u64) as usize).min(buf.len());
                reply.data(&buf[start..end]);
            }
            Err(_) => reply.error(Errno::EIO),
        }
    }
}

fn build(files: Vec<RpfFile>) -> RpfFs {
    let mut entries: HashMap<INodeNo, Entry> = HashMap::new();
    let mut children: HashMap<INodeNo, Vec<INodeNo>> = HashMap::new();
    children.entry(ROOT).or_default();

    let nf = files.len();
    let mut file_dir = vec![INodeNo(0); nf];
    let mut next_ino = 2u64;
    for f in 0..nf {
        let ino = INodeNo(next_ino);
        next_ino += 1;
        file_dir[f] = ino;
        entries.insert(
            ino,
            Entry {
                name: files[f].name.clone().into(),
                kind: FileType::Directory,
                stream: None,
            },
        );
        children.entry(ROOT).or_default().push(ino);
    }

    let mut fs = RpfFs { files, entries, children };
    for f in 0..fs.files.len() {
        let mut dir_inos = vec![INodeNo(0); fs.files[f].archives.len()];
        for a in 0..fs.files[f].archives.len() {
            let ino = INodeNo(next_ino);
            next_ino += 1;
            dir_inos[a] = ino;
            let archive_at = fs.files[f].archives[a].header.at;
            fs.entries.insert(
                ino,
                Entry { name: format!("h{archive_at:06x}").into(), kind: FileType::Directory, stream: None },
            );
            fs.children.entry(file_dir[f]).or_default().push(ino);
            let mut stream_idx = 0usize;
            for s in &fs.files[f].streams[a] {
                let ino = INodeNo(next_ino);
                next_ino += 1;
                fs.entries.insert(
                    ino,
                    Entry {
                        name: format!("{:08x}.{}", s.at, s.kind.label().0).into(),
                        kind: FileType::RegularFile,
                        stream: Some((f, a, stream_idx)),
                    },
                );
                fs.children.entry(dir_inos[a]).or_default().push(ino);
                stream_idx += 1;
            }
        }
        fs.files[f].dir_inos = dir_inos;
    }
    fs
}

fn main() -> ExitCode {
    let usage = "usage: rpf9-fuse <rpf> [mntpoint]\n";
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.len() > 2 {
        eprintln!("rpf9-fuse: too many arguments\n{usage}");
        return ExitCode::FAILURE;
    }
    if paths.is_empty() {
        eprintln!("rpf9-fuse: missing <rpf>\n{usage}");
        return ExitCode::FAILURE;
    }
    let Some(mntpoint) = paths.get(1) else {
        eprintln!("rpf9-fuse: missing [mntpoint]\n{usage}");
        return ExitCode::FAILURE;
    };

    let mut files = Vec::new();
    for path in paths.iter().take(1) {
        let f = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("rpf9-fuse: open {path}: {e}");
                return ExitCode::FAILURE;
            }
        };
        // SAFETY: the map is read-only; the file is not modified while mounted.
        let mmap = match unsafe { Mmap::map(&f) } {
            Ok(m) => m,
            Err(e) => {
                eprintln!("rpf9-fuse: map {path}: {e}");
                return ExitCode::FAILURE;
            }
        };
        let name = Path::new(path)
            .file_name()
            .map_or_else(|| path.clone(), |s| s.to_string_lossy().into_owned());
        let scan = Scan::new(&mmap);
        let mut streams = Vec::with_capacity(scan.archives.len());
        for i in 0..scan.archives.len() {
            streams.push(scan.owned_streams(i));
        }
        let mut dir_inos = Vec::with_capacity(scan.archives.len());
        for _ in 0..scan.archives.len() {
            dir_inos.push(INodeNo(0));
        }
        files.push(RpfFile { name, mmap, archives: scan.archives, streams, dir_inos });
    }

    let fs = build(files);
    match fuser::mount(fs, mntpoint, &Config::default()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rpf9-fuse: mount {}: {e}", mntpoint);
            ExitCode::FAILURE
        }
    }
}
