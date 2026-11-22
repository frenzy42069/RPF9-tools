//! Navmesh resources → triangle mesh (vertices + faces).
//!
//! Navmesh data lives in `navmeshes.rpf` as extracted `.rsc` resources. This
//! module discovers the mesh geometry in an extracted resource with
//! conservative heuristics:
//!
//! - **Vertices** — the longest run of consecutive f32 triples whose
//!   components are plausible world coordinates (`|v| <= 100000`, finite)
//!   and which contains the most "substantial" values (`|v| > 1`). The
//!   substantial preference avoids picking long near-zero regions, which are
//!   common in the archives.
//! - **Faces** — the longest run of index values that are all below the
//!   vertex count, in u16 or u32 form, preferring runs with nonzero values.
//!
//! Anything the heuristics cannot confirm (e.g. no face array is found) is
//! reported as empty rather than guessed.

#[derive(Debug, Default, Clone)]
pub struct Mesh {
    pub vertices: Vec<[f32; 3]>,
    pub faces: Vec<[u32; 3]>,
}

const MAX_COORD: f32 = 100_000.0;
const SUBSTANTIAL: f32 = 1.0;

fn f32_at(data: &[u8], at: usize) -> Option<f32> {
    data.get(at..at + 4)
        .and_then(|b| b.try_into().ok())
        .map(f32::from_le_bytes)
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    data.get(at..at + 2)
        .and_then(|b| b.try_into().ok())
        .map(u16::from_le_bytes)
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    data.get(at..at + 4)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes)
}

fn read_val(data: &[u8], at: usize, width: usize) -> Option<u32> {
    match width {
        2 => u16_at(data, at).map(|v| v as u32),
        _ => u32_at(data, at),
    }
}

fn plausible(v: f32) -> bool {
    v.is_finite() && v.abs() <= MAX_COORD
}

fn triple_plausible(data: &[u8], at: usize) -> bool {
    (0..3).all(|i| f32_at(data, at + 4 * i).is_some_and(plausible))
}

fn has_nonzero(data: &[u8], start: usize, triples: usize) -> bool {
    (0..triples).any(|i| {
        (0..3).any(|c| f32_at(data, start + 12 * i + 4 * c).is_some_and(|v| v != 0.0))
    })
}

fn count_substantial(data: &[u8], start: usize, triples: usize) -> usize {
    let mut n = 0;
    for i in 0..triples {
        for c in 0..3 {
            if f32_at(data, start + 12 * i + 4 * c).is_some_and(|v| v.abs() > SUBSTANTIAL) {
                n += 1;
            }
        }
    }
    n
}

/// Best run of consecutive plausible f32 triples: maximizes the number of
/// substantial values, then the run length. Returns `(start, triple_count)`.
fn find_vertex_run(data: &[u8]) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize, usize)> = None; // (start, len, substantial)
    let mut at = 0;
    while at + 12 <= data.len() {
        if !triple_plausible(data, at) {
            at += 4;
            continue;
        }
        let start = at;
        let mut n = 0;
        while at + 12 <= data.len() && triple_plausible(data, at) {
            n += 1;
            at += 12;
        }
        if n >= 3 && has_nonzero(data, start, n) {
            let substantial = count_substantial(data, start, n);
            let better = match &best {
                None => true,
                Some((_, bn, bs)) => substantial > *bs || (substantial == *bs && n > *bn),
            };
            if better {
                best = Some((start, n, substantial));
            }
        }
    }
    best.map(|(s, n, _)| (s, n))
}

enum FaceKind {
    U16,
    U32,
}

impl FaceKind {
    fn width(self) -> usize {
        match self {
            FaceKind::U16 => 2,
            FaceKind::U32 => 4,
        }
    }
}

/// Best run of index values: maximizes the number of nonzero values in the
/// run, then the run length. Returns `(start, value_count, nonzero_count)`.
fn best_index_run(data: &[u8], width: usize, ok: impl Fn(&[u8], usize) -> bool) -> Option<(usize, usize, usize)> {
    let mut best: Option<(usize, usize, usize)> = None; // (start, len, nonzero)
    let mut at = 0;
    while at + width <= data.len() {
        if !ok(data, at) {
            at += width;
            continue;
        }
        let start = at;
        let mut len = 0;
        let mut nonzero = 0;
        while at + width <= data.len() && ok(data, at) {
            len += 1;
            if read_val(data, at, width).is_some_and(|v| v != 0) {
                nonzero += 1;
            }
            at += width;
        }
        let better = match &best {
            None => true,
            Some((_, bl, bn)) => nonzero > *bn || (nonzero == *bn && len > *bl),
        };
        if better {
            best = Some((start, len, nonzero));
        }
    }
    best
}

fn find_face_run(data: &[u8], nverts: usize) -> Option<(usize, usize, FaceKind)> {
    let mut best: Option<(usize, usize, usize, FaceKind)> = None; // (start, len, nonzero, kind)
    let consider = |best: &mut Option<(usize, usize, usize, FaceKind)>,
                    r: (usize, usize, usize),
                    kind: FaceKind| {
        let better = match best {
            None => true,
            Some((_, bl, bn, _)) => r.2 > *bn || (r.2 == *bn && r.1 > *bl),
        };
        if better {
            *best = Some((r.0, r.1, r.2, kind));
        }
    };
    if nverts <= 65536 {
        if let Some(r) = best_index_run(data, 2, |d, o| {
            u16_at(d, o).is_some_and(|v| v as usize <= nverts)
        }) {
            consider(&mut best, r, FaceKind::U16);
        }
    }
    if let Some(r) = best_index_run(data, 4, |d, o| {
        u32_at(d, o).is_some_and(|v| v as usize <= nverts)
    }) {
        consider(&mut best, r, FaceKind::U32);
    }
    // A face run with no nonzero values is noise.
    best.filter(|(_, _, nz, _)| *nz >= 1)
        .map(|(s, l, _, k)| (s, l, k))
}

/// Parse the navmesh in `data`. Returns the discovered vertex/face arrays.
pub fn parse(data: &[u8]) -> Option<Mesh> {
    let (vstart, vn) = find_vertex_run(data)?;
    let vertices: Vec<[f32; 3]> = (0..vn)
        .map(|i| {
            let o = vstart + 12 * i;
            [
                f32_at(data, o).unwrap_or_default(),
                f32_at(data, o + 4).unwrap_or_default(),
                f32_at(data, o + 8).unwrap_or_default(),
            ]
        })
        .collect();
    let mut faces = Vec::new();
    if let Some((fstart, flen, kind)) = find_face_run(data, vertices.len()) {
        let width = kind.width();
        for i in 0..(flen / 3) {
            let base = fstart + width * 3 * i;
            let idx: Vec<u32> = (0..3).filter_map(|c| read_val(data, base + width * c, width)).collect();
            if idx.len() == 3 && idx.iter().all(|&v| (v as usize) < vertices.len()) {
                faces.push([idx[0], idx[1], idx[2]]);
            }
        }
    }
    Some(Mesh { vertices, faces })
}

/// Write the mesh as a Wavefront OBJ document.
pub fn write_obj(out: &mut impl std::io::Write, mesh: &Mesh) -> std::io::Result<()> {
    writeln!(
        out,
        "# navmesh: {} vertices, {} faces",
        mesh.vertices.len(),
        mesh.faces.len()
    )?;
    for v in &mesh.vertices {
        writeln!(out, "v {} {} {}", v[0], v[1], v[2])?;
    }
    for f in &mesh.faces {
        writeln!(out, "f {} {} {}", f[0] + 1, f[1] + 1, f[2] + 1)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_f32(buf: &mut Vec<u8>, v: f32) {
        buf.extend_from_slice(&v.to_le_bytes());
    }

    #[test]
    fn parses_synthetic_mesh() {
        let mut d = Vec::new();
        for t in [(0.0f32, 0.0, 0.0), (1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0)] {
            push_f32(&mut d, t.0);
            push_f32(&mut d, t.1);
            push_f32(&mut d, t.2);
        }
        // faces: u16 triples [0,1,2], [0,2,3]
        d.extend_from_slice(&0u16.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&2u16.to_le_bytes());
        d.extend_from_slice(&0u16.to_le_bytes());
        d.extend_from_slice(&2u16.to_le_bytes());
        d.extend_from_slice(&3u16.to_le_bytes());

        let mesh = parse(&d).expect("parse");
        assert!(mesh.vertices.len() >= 4, "got {} vertices", mesh.vertices.len());
        assert_eq!(mesh.faces, vec![[0, 1, 2], [0, 2, 3]]);
    }

    #[test]
    fn parses_real_navmesh_file() {
        let path = "../results/navmeshes/h125307000/1616ff010.rsc";
        let Ok(d) = std::fs::read(path) else {
            return; // results/ not extracted yet — covered by the synthetic test.
        };
        let mesh = parse(&d).expect("parse");
        assert!(mesh.vertices.len() >= 3, "got {} vertices", mesh.vertices.len());
        // The heuristic picks the run with the most substantial coordinates;
        // verify a few of the observed values around it.
        let found = mesh.vertices.iter().any(|v| v[0].abs() > 10000.0);
        assert!(found, "no substantial coordinates in {:?}", mesh.vertices.first());
    }
}
