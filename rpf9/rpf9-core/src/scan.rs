//! Full scan: walk archives, record streams, group by owner.

use crate::archive::{find_archives, Archive};
use crate::media::carve_media;
use crate::stream::{walk, Stream};

pub struct Scan {
    pub archives: Vec<Archive>,
    pub streams: Vec<Stream>,
}

impl Scan {
    pub fn new(data: &[u8]) -> Scan {
        let archives = find_archives(data);
        let mut streams = Vec::new();
        for a in &archives {
            streams.extend(walk(data, a.body.start, a.trailer.start));
        }
        // Carve media in the archive body gaps (movies, banks, images).
        let covered: Vec<_> = {
            let mut c = streams.iter().map(|s| s.at..s.at + s.stored_len).collect::<Vec<_>>();
            c.sort_by_key(|r| r.start);
            c
        };
        for a in &archives {
            streams.extend(carve_media(data, a.body.start..a.trailer.start, &covered));
        }
        streams.sort_by_key(|s| s.at);
        Scan { archives, streams }
    }

    /// The innermost archive containing `at` (offs `owner`).
    pub fn owner(&self, at: usize) -> usize {
        self.archives
            .iter()
            .rposition(|a| a.extent().contains(&at))
            .unwrap_or(0)
    }

    /// Streams owned by `archive_idx` — each stream is owned by the innermost
    /// archive containing it, so nested archives report only their own.
    pub fn owned_streams(&self, archive_idx: usize) -> Vec<Stream> {
        self.streams
            .iter()
            .filter(|s| self.owner(s.at) == archive_idx)
            .copied()
            .collect()
    }

    pub fn total_streams(&self) -> usize {
        self.streams.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game_path(name: &str) -> String {
        format!("{}/../../game/{name}", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn scan_appdata0() {
        let data = std::fs::read(game_path("appdata0.rpf")).unwrap();
        let scan = Scan::new(&data);
        assert_eq!(scan.archives.len(), 1);
        let a = &scan.archives[0];
        // doc §2: appdata0 header: A=0x360, B=0x340, C=0x40, N=2.
        assert_eq!(a.header.len, 0x360);
        assert_eq!(a.header.split, 0x340);
        assert_eq!(a.header.trailer_len, 0x40);
        assert_eq!(a.header.files, 2);
        assert_eq!(scan.owned_streams(0).len(), 0);
    }

    #[test]
    fn scan_common_rpf() {
        let data = std::fs::read(game_path("common.rpf")).unwrap();
        let scan = Scan::new(&data);
        assert!(!scan.archives.is_empty());
        // doc: "common contains 954 valid zlib streams, 679 of them 64 KiB;
        // 259 complete XML files."
        let streams = scan.owned_streams(0);
        assert!(streams.len() >= 954, "got {} streams", streams.len());
        let xml = streams.iter().filter(|s| s.kind == crate::kinds::Kind::Xml).count();
        assert!(xml >= 259, "got {xml} XML streams");
    }

    #[test]
    fn scan_script_children() {
        let data = std::fs::read(game_path("script.rpf")).unwrap();
        let scan = Scan::new(&data);
        // doc: outer + nested children at 0x1000 and 0xC4000.
        assert!(scan.archives.len() >= 3, "got {} archives", scan.archives.len());
        let child = scan
            .archives
            .iter()
            .find(|a| a.header.at == 0xC4000)
            .expect("nested script child at 0xC4000");
        assert_eq!(child.header.len, 0x11F10);
        assert_eq!(child.header.split, 0xEF90);
        assert_eq!(child.header.trailer_len, 0x62B0);
        assert_eq!(child.header.files, 1012);
    }

    #[test]
    fn scan_data_rpf_child() {
        let data = std::fs::read(game_path("data.rpf")).unwrap();
        let scan = Scan::new(&data);
        // doc §6/§11: 13 mode-2 children, all with N=1115, C=0x74A0; the
        // child at 0x11de000 has a PSIN at +0x14000.
        let idx = scan
            .archives
            .iter()
            .position(|a| a.header.at == 0x11de000)
            .expect("child at 0x11de000");
        let child = &scan.archives[idx];
        assert_eq!(child.header.len, 0x133B0);
        assert_eq!(child.header.split, 0xFF60);
        assert_eq!(child.header.trailer_len, 0x74A0);
        assert_eq!(child.header.files, 1115);
        assert!(child.header.id != 0);
        let streams = scan.owned_streams(idx);
        assert!(streams.len() >= 1115, "got {} streams", streams.len());
        assert!(
            streams
                .iter()
                .any(|s| s.at == 0x11f2000 && s.kind == crate::kinds::Kind::PsIn)
        );
    }

    #[test]
    fn scan_data_rpf_outer() {
        let data = std::fs::read(game_path("data.rpf")).unwrap();
        let scan = Scan::new(&data);
        let outer = &scan.archives[0];
        assert_eq!(outer.header.at, 0);
        assert_eq!(outer.header.len, 0x11060);
        assert_eq!(outer.header.files, 699);
    }
}
