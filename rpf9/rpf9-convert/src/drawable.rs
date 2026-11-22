//! Decoder for the packed drawable layout observed in levels.rpf.

use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::Path,
};

use anyhow::{ensure, Context, Result};
use flate2::bufread::ZlibDecoder;

use crate::mesh::Mesh;

const CPU_BASE: u32 = 0x5000_0000;
const GPU_BASE: u32 = 0x6000_0000;
const PAGE_LIMIT: u64 = 64 * 1024 * 1024;

pub(crate) fn load(path: &Path, offset: u64) -> Result<Mesh> {
    load_named(path, offset).map(|(_, mesh)| mesh)
}

pub(crate) fn load_named(path: &Path, offset: u64) -> Result<(String, Mesh)> {
    let mut reader = BufReader::new(File::open(path)?);
    reader.seek(SeekFrom::Start(offset))?;
    let cpu = read_page(&mut reader).context("decode drawable system stream")?;
    let end = reader.stream_position()?;
    let relative_end = end.checked_sub(offset).context("invalid stream position")?;
    let padding = (16 - relative_end % 16) % 16;
    let mut pad = [0; 15];
    reader.read_exact(&mut pad[..padding as usize])?;
    ensure!(
        pad == [0; 15],
        "expected zero padding before graphics stream"
    );
    let mut gpu = read_page(&mut reader).context("decode drawable graphics stream")?;
    let name = drawable_name(&cpu)?.to_owned();
    loop {
        match decode(&cpu, &gpu) {
            Ok(mesh) => return Ok((name, mesh)),
            Err(error)
                if error.chain().any(|cause| {
                    matches!(
                        cause.to_string().as_str(),
                        "truncated drawable"
                            | "truncated drawable positions"
                            | "truncated meshlet vertex map"
                    )
                }) && gpu.len().is_multiple_of(65536)
                    && (gpu.len() as u64) < PAGE_LIMIT =>
            {
                let end = reader.stream_position()?;
                let padding = (16 - (end - offset) % 16) % 16;
                let mut bytes = [0; 15];
                reader.read_exact(&mut bytes[..padding as usize])?;
                ensure!(
                    bytes == [0; 15],
                    "expected zero padding before graphics continuation"
                );
                let continuation =
                    read_page(&mut reader).context("decode graphics continuation")?;
                ensure!(
                    !continuation.is_empty()
                        && gpu.len() + continuation.len() <= PAGE_LIMIT as usize,
                    "invalid graphics continuation length"
                );
                ensure!(
                    !(continuation.starts_with(&[0; 8])
                        && address(&continuation, 8, CPU_BASE).is_ok()),
                    "graphics continuation points to a new resource"
                );
                gpu.extend_from_slice(&continuation);
            }
            Err(error) => return Err(error),
        }
    }
}

fn read_page(reader: &mut impl BufRead) -> Result<Vec<u8>> {
    let mut page = Vec::new();
    ZlibDecoder::new(reader)
        .take(PAGE_LIMIT + 1)
        .read_to_end(&mut page)?;
    ensure!(
        page.len() as u64 <= PAGE_LIMIT,
        "drawable stream is too large"
    );
    Ok(page)
}

fn word(bytes: &[u8], offset: usize) -> Result<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .context("truncated drawable")?;
    Ok(u32::from_le_bytes(value.try_into()?))
}

fn address(bytes: &[u8], offset: usize, base: u32) -> Result<usize> {
    ensure!(word(bytes, offset + 4)? == 0, "invalid drawable pointer");
    let address = word(bytes, offset)?;
    let relative = address
        .checked_sub(base)
        .context("invalid drawable pointer")?;
    ensure!(relative < 0x1000_0000, "invalid drawable pointer");
    Ok(relative as usize)
}

fn drawable_name(cpu: &[u8]) -> Result<&str> {
    rpf9_core::names::drawable_name(cpu).context("expected a named RAGE drawable")
}

fn decode(cpu: &[u8], gpu: &[u8]) -> Result<Mesh> {
    drawable_name(cpu)?;

    let mut mesh = Mesh {
        positions: Vec::new(),
        indices: Vec::new(),
        normals: None,
        texcoords: None,
    };
    for geometry in (0x10..cpu.len().saturating_sub(0x30)).step_by(8) {
        if word(cpu, geometry + 0x10)? == 0x1601 && address(cpu, geometry + 0x18, GPU_BASE).is_ok()
        {
            decode_geometry(cpu, gpu, geometry, &mut mesh)
                .with_context(|| format!("decode geometry at {geometry:#x}"))?;
        }
    }
    ensure!(
        !mesh.indices.is_empty(),
        "drawable has no supported geometry"
    );
    Ok(mesh)
}

fn decode_geometry(cpu: &[u8], gpu: &[u8], geometry: usize, mesh: &mut Mesh) -> Result<()> {
    let attributes = address(cpu, geometry + 0x18, GPU_BASE)?;
    let references = address(cpu, geometry - 0x10, CPU_BASE)?;
    let meshlet_info = address(cpu, references, CPU_BASE)?;
    let meshlets = address(cpu, meshlet_info + 0x18, GPU_BASE)?;
    let flags = word(gpu, meshlets)?;
    ensure!(
        flags & 0xffff_0000 == 0x8000_0000,
        "unsupported meshlet vertex map format"
    );
    let count = (flags & 0xffff) as usize;
    let face_count = word(gpu, meshlets + 4)? as usize;
    let remap_count = word(gpu, meshlets + 8)? as usize;
    ensure!(
        count > 0 && (1..=65536).contains(&face_count) && (3..=65536).contains(&remap_count),
        "invalid meshlet counts"
    );

    // The observed 0x42 position stream starts at +0x80. Its components
    // are unsigned 24/24/16-bit values with per-axis biases and scales.
    ensure!(
        word(gpu, attributes)? == 0x4042,
        "unsupported position stream"
    );
    ensure!(
        word(gpu, attributes + 0x5c)? == 0x200 && word(gpu, attributes + 0x6c)? == 2,
        "unsupported position quantization"
    );
    let vec3 = |start| -> Result<[f32; 3]> {
        Ok([
            f32::from_bits(word(gpu, start)?),
            f32::from_bits(word(gpu, start + 4)?),
            f32::from_bits(word(gpu, start + 8)?),
        ])
    };
    let bias = vec3(attributes + 0x60)?;
    let scale = vec3(attributes + 0x70)?;
    ensure!(
        bias.iter().all(|v| v.is_finite()) && scale.iter().all(|v| v.is_finite() && *v > 0.0),
        "invalid drawable position quantization"
    );

    let normal_stream = word(gpu, attributes + 8)?;
    ensure!(
        normal_stream & 255 == 0x21,
        "unsupported position stream bounds"
    );
    let position_end = ((normal_stream >> 8) * 2) as usize;
    ensure!(
        position_end >= 0x80 && (position_end - 0x80).is_multiple_of(8),
        "invalid position stream bounds"
    );
    let vertices = (position_end - 0x80) / 8;
    let vertex_data = gpu
        .get(attributes + 0x80..attributes + position_end)
        .context("truncated drawable positions")?;
    let base = u32::try_from(mesh.positions.len())?;
    ensure!(
        base.checked_add(u32::try_from(vertices)?).is_some(),
        "too many vertices"
    );
    // Attribute 0x28 stores two signed fixed-point coordinates per vertex.
    // The source uses a top-left UV origin, while Mesh follows OBJ convention.
    let uv_offset = (0..0x5c).step_by(4).find_map(|offset| {
        let descriptor = word(gpu, attributes + offset).ok()?;
        (descriptor & 0xff == 0x28).then_some(((descriptor >> 8) as usize) * 2)
    });
    let uv_data = uv_offset.and_then(|offset| {
        let start = attributes.checked_add(offset)?;
        let end = start.checked_add(vertices.checked_mul(4)?)?;
        (offset >= position_end)
            .then(|| gpu.get(start..end))
            .flatten()
    });
    let previous_vertices = mesh.positions.len();
    match (mesh.texcoords.as_mut(), uv_data) {
        (Some(uvs), Some(data)) => append_uvs(uvs, data),
        (None, Some(data)) if previous_vertices == 0 => {
            let mut uvs = Vec::with_capacity(vertices);
            append_uvs(&mut uvs, data);
            mesh.texcoords = Some(uvs);
        }
        _ => mesh.texcoords = None,
    }

    for packed in vertex_data.as_chunks::<8>().0 {
        let x = u32::from_le_bytes([packed[1], packed[2], packed[3], 0]);
        let y = u32::from_le_bytes([packed[0], packed[4], packed[5], 0]);
        let z = u32::from(u16::from_le_bytes([packed[6], packed[7]]));
        let [x, y, z] = [x, y, z].map(|value| value as f32);
        mesh.positions.push([
            bias[0] + x * scale[0],
            bias[2] + z * scale[2],
            -(bias[1] + y * scale[1]),
        ]);
    }

    let descriptors = meshlets + 12;
    let triangle_start = descriptors + count * 16;
    let remap_start = triangle_start + face_count * 4;
    let remaps = gpu
        .get(remap_start..remap_start + remap_count * 2)
        .context("truncated meshlet vertex map")?
        .as_chunks::<2>()
        .0;
    let mut faces = 0;
    let mut entries = 0;
    for id in 0..count {
        let descriptor = descriptors + id * 16;
        let counts = word(gpu, descriptor)?;
        let offsets = word(gpu, descriptor + 4)?;
        let local_vertices = ((counts & 127) + 1) as usize;
        let local_faces = (((counts >> 7) & 127) + 1) as usize;
        ensure!(
            offsets as u16 as usize == entries
                && (offsets >> 16) as usize == faces
                && entries + local_vertices <= remap_count
                && faces + local_faces <= face_count,
            "invalid meshlet ranges"
        );
        let map = &remaps[entries..entries + local_vertices];
        for index in faces..faces + local_faces {
            let packed = word(gpu, triangle_start + index * 4)?;
            ensure!(
                (packed >> 21) as usize == id,
                "unsupported meshlet triangle encoding"
            );
            for shift in [0, 7, 14] {
                let local = ((packed >> shift) & 127) as usize;
                let &global = map.get(local).context("invalid meshlet triangle index")?;
                let vertex = u16::from_le_bytes(global);
                ensure!((vertex as usize) < vertices, "invalid meshlet vertex map");
                mesh.indices.push(base + u32::from(vertex));
            }
        }
        faces += local_faces;
        entries += local_vertices;
    }
    ensure!(
        faces == face_count && entries == remap_count,
        "incomplete meshlets"
    );
    Ok(())
}

fn append_uvs(uvs: &mut Vec<[f32; 2]>, data: &[u8]) {
    for bytes in data.as_chunks::<4>().0 {
        let u = i16::from_le_bytes([bytes[0], bytes[1]]) as f32 / 8192.0;
        let v = i16::from_le_bytes([bytes[2], bytes[3]]) as f32 / 8192.0;
        uvs.push([u, 1.0 - v]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn decodes_signed_fixed_point_uvs() {
        let mut uvs = Vec::new();
        append_uvs(&mut uvs, &[0, 0, 0, 32, 0, 32, 0, 0, 0, 224, 0, 16]);
        assert_eq!(uvs, vec![[0.0, 0.0], [1.0, 1.0], [-1.0, 0.5]]);
    }

    #[test]
    fn decodes_local_drawable_with_graphics_continuation_when_available() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game/levels.rpf");
        if !path.is_file() {
            return;
        }
        let (name, mesh) = load_named(&path, 0xf9c8b010).unwrap();
        assert_eq!(name, "int_abd_1_laundry_details_ceiling_panels.#dr");
        assert!(!mesh.positions.is_empty());
        assert!(!mesh.indices.is_empty());
    }

    #[test]
    fn decodes_uvs_on_local_drawable_when_available() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game/levels.rpf");
        if !path.is_file() {
            return;
        }
        let mesh = load(&path, 0xf9c9d010).unwrap();
        assert_eq!(
            mesh.texcoords.unwrap(),
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
        );
    }
}
