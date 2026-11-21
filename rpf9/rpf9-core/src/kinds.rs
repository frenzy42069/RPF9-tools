//! Classification of recovered resources by their decompressed signature.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// zlib stream, unparsed
    Resource,
    /// "PSIN" — RPF9 resource manifest
    PsIn,
    /// XML document
    Xml,
    /// "0DRP" — RAGE 9 meshlet resource
    Rsc9,
    /// "HMAP" — height map
    Hmap,
    /// "MHIU" — UI map
    Uihm,
    /// "SPS1" — shader catalogue
    Sps1,
    /// "TTPF" — texture database
    Ttpf,
    /// "ETPF" — texture database (EAGLE variant)
    Ettpf,
    /// "ADAT" — audio bank
    Adat,
    /// Bink2 ("KB2"/"BINK") movie
    Movie,
    /// Carved PNG image
    Png,
    /// Carved RIFF file ("WAVE" audio, "AVI " video, "WEBP" image)
    Riff { ext: &'static str, name: &'static str },
    /// "PS\0\x09" — Xbox Series X PSO cache
    Shaders,
}

impl Kind {
    /// (extension, kind label) — matches offs/rpf9-extract's naming style.
    pub fn label(&self) -> (&'static str, &'static str) {
        match self {
            Kind::Resource => ("rsc", "resource"),
            Kind::PsIn => ("psin", "PSIN catalog"),
            Kind::Xml => ("xml", "XML"),
            Kind::Rsc9 => ("rsc9", "meshlet resource"),
            Kind::Hmap => ("hmap", "HMAP height map"),
            Kind::Uihm => ("uihm", "UIHM map"),
            Kind::Sps1 => ("sps1", "SPS1 shader catalog"),
            Kind::Ttpf => ("ttpf", "texture database"),
            Kind::Ettpf => ("etpf", "texture database"),
            Kind::Adat => ("awc", "ADAT audio bank"),
            Kind::Movie => ("bk2", "movie"),
            Kind::Png => ("png", "PNG image"),
            Kind::Riff { ext, name } => (*ext, *name),
            Kind::Shaders => ("ps09", "PS\\x00\\x09 shader cache"),
        }
    }

    /// Carved media kinds (stored raw, not zlib).
    pub fn is_media(&self) -> bool {
        matches!(self, Kind::Movie | Kind::Adat | Kind::Png | Kind::Riff { .. })
    }
}

/// Classify by the first bytes of a decompressed stream.
pub fn classify(out: &[u8]) -> Kind {
    if out.get(..4) == Some(b"PSIN") {
        return Kind::PsIn;
    }
    // doc: "An object start requires eight zero bytes and 0DRP at +0x10";
    // in practice the decompressed head is zeros then "0DRP".
    if out.len() >= 20 {
        for i in 12..20 {
            if out.get(i..i + 4) == Some(b"0DRP") {
                return Kind::Rsc9;
            }
        }
    }
    // XML, including a UTF-8 BOM prefix (doc §16: "257 with a declaration,
    // including BOM").
    let body = match out.strip_prefix(b"\xEF\xBB\xBF") {
        Some(b) => b,
        None => out,
    };
    if body.first() == Some(&b'<') {
        return Kind::Xml;
    }
    match out.first() {
        Some(b'P') if out.starts_with(b"PS\x00\x09") => Kind::Shaders,
        _ => {
            if out.starts_with(b"TTPF") {
                Kind::Ttpf
            } else if out.starts_with(b"ETPF") {
                Kind::Ettpf
            } else if out.starts_with(b"HMAP") {
                Kind::Hmap
            } else if out.starts_with(b"MHIU") {
                Kind::Uihm
            } else if out.starts_with(b"SPS1") {
                Kind::Sps1
            } else if out.starts_with(b"ADAT") {
                Kind::Adat
            } else {
                Kind::Resource
            }
        }
    }
}
