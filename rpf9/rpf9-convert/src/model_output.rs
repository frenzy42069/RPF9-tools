use std::io::Write;

use anyhow::{ensure, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use clap::ValueEnum;
use serde_json::{json, Value};

use crate::mesh::Mesh;

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    Glb,
    Gltf,
    Obj,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Glb => "glb",
            Self::Gltf => "gltf",
            Self::Obj => "obj",
        }
    }
}

pub(crate) fn write(output: &mut impl Write, mesh: &Mesh, format: Format) -> Result<()> {
    validate(mesh)?;
    match format {
        Format::Glb | Format::Gltf => write_gltf(output, mesh, format),
        Format::Obj => write_obj(output, mesh),
    }
}

fn validate(mesh: &Mesh) -> Result<()> {
    let count = mesh.positions.len();
    ensure!(count > 0, "model has no vertices");
    ensure!(
        !mesh.indices.is_empty() && mesh.indices.as_chunks::<3>().1.is_empty(),
        "model has no complete triangles"
    );
    ensure!(
        mesh.indices.iter().all(|&index| (index as usize) < count),
        "triangle references a missing vertex"
    );
    ensure!(
        mesh.positions
            .iter()
            .flatten()
            .all(|value| value.is_finite()),
        "model has non-finite positions"
    );
    if let Some(normals) = &mesh.normals {
        ensure!(
            normals.len() == count && normals.iter().flatten().all(|value| value.is_finite()),
            "invalid vertex normals"
        );
    }
    if let Some(texcoords) = &mesh.texcoords {
        ensure!(
            texcoords.len() == count && texcoords.iter().flatten().all(|value| value.is_finite()),
            "invalid texture coordinates"
        );
    }
    Ok(())
}

#[derive(Default)]
struct GltfBuffer {
    bytes: Vec<u8>,
    accessors: Vec<Value>,
    views: Vec<Value>,
}

impl GltfBuffer {
    fn append(
        &mut self,
        values: impl Iterator<Item = [u8; 4]>,
        count: usize,
        kind: &str,
        component: u32,
        target: u32,
    ) -> usize {
        let offset = self.bytes.len();
        self.bytes.extend(values.flatten());
        let index = self.views.len();
        self.views.push(json!({
            "buffer": 0, "byteOffset": offset,
            "byteLength": self.bytes.len() - offset, "target": target,
        }));
        self.accessors.push(json!({
            "bufferView": index, "componentType": component, "count": count, "type": kind,
        }));
        index
    }

    fn vertex_attribute<const N: usize>(&mut self, vertices: &[[f32; N]], kind: &str) -> usize {
        self.append(
            vertices.iter().flatten().map(|v| v.to_le_bytes()),
            vertices.len(),
            kind,
            5126,
            34962,
        )
    }
}

fn write_gltf(output: &mut impl Write, mesh: &Mesh, format: Format) -> Result<()> {
    let mut buffer = GltfBuffer::default();
    let position = buffer.vertex_attribute(&mesh.positions, "VEC3");
    let mut min = mesh.positions[0];
    let mut max = min;
    for vertex in &mesh.positions {
        for axis in 0..3 {
            min[axis] = min[axis].min(vertex[axis]);
            max[axis] = max[axis].max(vertex[axis]);
        }
    }
    buffer.accessors[position]["min"] = json!(min);
    buffer.accessors[position]["max"] = json!(max);
    let mut attributes = json!({ "POSITION": position });
    if let Some(normals) = &mesh.normals {
        let normalized: Vec<_> = normals
            .iter()
            .map(|&[x, y, z]| {
                let length = f64::from(x).hypot(f64::from(y)).hypot(f64::from(z));
                if length == 0.0 {
                    [0.0, 0.0, 1.0]
                } else {
                    [x, y, z].map(|component| (f64::from(component) / length) as f32)
                }
            })
            .collect();
        attributes["NORMAL"] = json!(buffer.vertex_attribute(&normalized, "VEC3"));
    }
    if let Some(texcoords) = &mesh.texcoords {
        let flipped: Vec<_> = texcoords.iter().map(|&[u, v]| [u, 1.0 - v]).collect();
        attributes["TEXCOORD_0"] = json!(buffer.vertex_attribute(&flipped, "VEC2"));
    }
    let indices = buffer.append(
        mesh.indices.iter().map(|v| v.to_le_bytes()),
        mesh.indices.len(),
        "SCALAR",
        5125,
        34963,
    );

    let mut document = json!({
        "asset": { "version": "2.0", "generator": "rpf9-convert" },
        "scene": 0, "scenes": [{ "nodes": [0] }], "nodes": [{ "mesh": 0 }],
        "meshes": [{ "primitives": [{ "attributes": attributes, "indices": indices }] }],
        "buffers": [{ "byteLength": buffer.bytes.len() }],
        "bufferViews": buffer.views, "accessors": buffer.accessors,
    });

    if matches!(format, Format::Gltf) {
        document["buffers"][0]["uri"] = json!(format!(
            "data:application/octet-stream;base64,{}",
            STANDARD.encode(buffer.bytes)
        ));
        serde_json::to_writer_pretty(output, &document)?;
        return Ok(());
    }

    let mut json_bytes = serde_json::to_vec(&document)?;
    json_bytes.resize(json_bytes.len().next_multiple_of(4), b' ');
    let json_length = u32::try_from(json_bytes.len())?;
    let bin_length = u32::try_from(buffer.bytes.len())?;
    let length = json_length
        .checked_add(bin_length)
        .and_then(|size| size.checked_add(28))
        .ok_or_else(|| anyhow::anyhow!("model exceeds the GLB 4 GiB size limit"))?;

    output.write_all(b"glTF")?;
    output.write_all(&2_u32.to_le_bytes())?;
    output.write_all(&length.to_le_bytes())?;
    output.write_all(&json_length.to_le_bytes())?;
    output.write_all(b"JSON")?;
    output.write_all(&json_bytes)?;
    output.write_all(&bin_length.to_le_bytes())?;
    output.write_all(b"BIN\0")?;
    output.write_all(&buffer.bytes)?;
    Ok(())
}

fn write_obj(output: &mut impl Write, mesh: &Mesh) -> Result<()> {
    for [x, y, z] in &mesh.positions {
        writeln!(output, "v {x} {y} {z}")?;
    }
    if let Some(texcoords) = &mesh.texcoords {
        for [u, v] in texcoords {
            writeln!(output, "vt {u} {v}")?;
        }
    }
    if let Some(normals) = &mesh.normals {
        for [x, y, z] in normals {
            writeln!(output, "vn {x} {y} {z}")?;
        }
    }
    for triangle in mesh.indices.as_chunks::<3>().0 {
        write!(output, "f")?;
        for &index in triangle {
            let n = u64::from(index) + 1;
            match (mesh.texcoords.is_some(), mesh.normals.is_some()) {
                (true, true) => write!(output, " {n}/{n}/{n}")?,
                (true, false) => write!(output, " {n}/{n}")?,
                (false, true) => write!(output, " {n}//{n}")?,
                (false, false) => write!(output, " {n}")?,
            }
        }
        writeln!(output)?;
    }
    Ok(())
}
