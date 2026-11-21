use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::Path;

pub(crate) struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub normals: Option<Vec<[f32; 3]>>,
    pub texcoords: Option<Vec<[f32; 2]>>,
}

/// Read a Wavefront OBJ mesh. Native RAGE model resources require a separate decoder.
pub(crate) fn load(path: &Path) -> Result<Mesh> {
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("obj"))
    {
        bail!("unsupported model input: expected Wavefront OBJ or use --offset to select a supported RAGE drawable");
    }
    let data = std::fs::read_to_string(path)
        .with_context(|| format!("reading model {}", path.display()))?;
    parse_obj(&data).with_context(|| format!("parsing model {}", path.display()))
}

fn parse_obj(data: &str) -> Result<Mesh> {
    let mut source_positions = Vec::new();
    let mut source_normals = Vec::new();
    let mut source_texcoords = Vec::new();
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut texcoords = Vec::new();
    let mut indices = Vec::new();
    let mut vertices = HashMap::new();
    let mut attribute_layout = None;

    for (line_number, line) in data.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("");
        let mut fields = line.split_whitespace();
        let Some(kind) = fields.next() else { continue };
        let result = (|| -> Result<()> {
            match kind {
                "v" => source_positions.push(parse_vector::<3>(&mut fields)?),
                "vn" => source_normals.push(parse_vector::<3>(&mut fields)?),
                "vt" => source_texcoords.push(parse_vector::<2>(&mut fields)?),
                "f" => {
                    let mut face = Vec::new();
                    for field in fields {
                        let mut parts = field.split('/');
                        let position =
                            obj_index(parts.next().unwrap_or(""), source_positions.len())?;
                        let texture = optional_index(parts.next(), source_texcoords.len())?;
                        let normal = optional_index(parts.next(), source_normals.len())?;
                        if parts.next().is_some() {
                            bail!("too many indices in face vertex {field}");
                        }
                        let layout = (texture.is_some(), normal.is_some());
                        if let Some(existing) = attribute_layout {
                            if layout != existing {
                                bail!(
                                    "face vertices use inconsistent texture coordinates or normals"
                                );
                            }
                        } else {
                            attribute_layout = Some(layout);
                        }
                        let key = (position, texture, normal);
                        let index = if let Some(&index) = vertices.get(&key) {
                            index
                        } else {
                            let index =
                                u32::try_from(positions.len()).context("too many vertices")?;
                            positions.push(source_positions[position]);
                            if let Some(texture) = texture {
                                texcoords.push(source_texcoords[texture]);
                            }
                            if let Some(normal) = normal {
                                normals.push(source_normals[normal]);
                            }
                            vertices.insert(key, index);
                            index
                        };
                        face.push(index);
                    }
                    if face.len() < 3 {
                        bail!("face has fewer than three vertices");
                    }
                    if face.len() > 3 && !is_convex(&face, &positions) {
                        bail!("non-convex or degenerate OBJ polygon must be triangulated before conversion");
                    }
                    for i in 1..face.len() - 1 {
                        indices.extend_from_slice(&[face[0], face[i], face[i + 1]]);
                    }
                }
                // Object names, groups, smoothing and materials do not alter geometry.
                _ => {}
            }
            Ok(())
        })();
        result.with_context(|| format!("OBJ line {}", line_number + 1))?;
    }
    if indices.is_empty() {
        bail!("OBJ contains no faces");
    }
    let (has_texcoords, has_normals) = attribute_layout.unwrap_or((false, false));
    Ok(Mesh {
        positions,
        indices,
        normals: has_normals.then_some(normals),
        texcoords: has_texcoords.then_some(texcoords),
    })
}

fn is_convex(face: &[u32], positions: &[[f32; 3]]) -> bool {
    let mut normal = [0.0f64; 3];
    for i in 0..face.len() {
        let a = positions[face[i] as usize];
        let b = positions[face[(i + 1) % face.len()] as usize];
        normal[0] += f64::from(a[1] - b[1]) * f64::from(a[2] + b[2]);
        normal[1] += f64::from(a[2] - b[2]) * f64::from(a[0] + b[0]);
        normal[2] += f64::from(a[0] - b[0]) * f64::from(a[1] + b[1]);
    }
    let mut direction = 0.0f64;
    for i in 0..face.len() {
        let a = positions[face[i] as usize];
        let b = positions[face[(i + 1) % face.len()] as usize];
        let c = positions[face[(i + 2) % face.len()] as usize];
        let ab = [
            f64::from(b[0] - a[0]),
            f64::from(b[1] - a[1]),
            f64::from(b[2] - a[2]),
        ];
        let bc = [
            f64::from(c[0] - b[0]),
            f64::from(c[1] - b[1]),
            f64::from(c[2] - b[2]),
        ];
        let cross = [
            ab[1] * bc[2] - ab[2] * bc[1],
            ab[2] * bc[0] - ab[0] * bc[2],
            ab[0] * bc[1] - ab[1] * bc[0],
        ];
        let dot = cross.iter().zip(normal).map(|(a, b)| a * b).sum::<f64>();
        if dot != 0.0 {
            if direction != 0.0 && dot.signum() != direction {
                return false;
            }
            direction = dot.signum();
        }
    }
    direction != 0.0
}

fn parse_vector<const N: usize>(fields: &mut std::str::SplitWhitespace<'_>) -> Result<[f32; N]> {
    let mut vector = [0.0_f32; N];
    for value in &mut vector {
        *value = fields
            .next()
            .context("missing vector component")?
            .parse()
            .context("invalid vector component")?;
        if !value.is_finite() {
            bail!("non-finite vector component");
        }
    }
    Ok(vector)
}

fn optional_index(value: Option<&str>, count: usize) -> Result<Option<usize>> {
    match value {
        None | Some("") => Ok(None),
        Some(value) => obj_index(value, count).map(Some),
    }
}

fn obj_index(value: &str, count: usize) -> Result<usize> {
    let index: i64 = value.parse().context("invalid OBJ index")?;
    let count = i64::try_from(count).context("too many OBJ values")?;
    let resolved = if index > 0 { index - 1 } else { count + index };
    if index == 0 || resolved < 0 || resolved >= count {
        bail!("OBJ index {index} is out of range for {count} values");
    }
    Ok(resolved as usize)
}

#[cfg(test)]
mod tests {
    use super::parse_obj;

    #[test]
    fn triangulates_and_keeps_separate_corner_attributes() {
        let mesh = parse_obj("v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvn 0 0 1\nf -4/1/1 -3/2/1 -2/3/1 -1/4/1\n").unwrap();
        assert_eq!(mesh.indices, [0, 1, 2, 0, 2, 3]);
        assert_eq!(mesh.positions.len(), 4);
        assert_eq!(mesh.texcoords.unwrap().len(), 4);
        assert_eq!(mesh.normals.unwrap().len(), 4);
    }

    #[test]
    fn splits_vertices_at_uv_seams_and_rejects_concave_faces() {
        let mesh = parse_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvt 1 1\nf 1/1 2/2 3/3\nf 1/4 2/2 3/3\n").unwrap();
        assert_eq!(mesh.positions.len(), 4);
        assert_ne!(mesh.indices[0], mesh.indices[3]);
        assert!(parse_obj("v 0 0 0\nv 2 0 0\nv 1 1 0\nv 2 2 0\nv 0 2 0\nf 1 2 3 4 5\n").is_err());
    }

    #[test]
    fn rejects_invalid_indices_and_mixed_attributes() {
        assert!(parse_obj("v 0 0 0\nf 1 1 0\n").is_err());
        assert!(parse_obj("v 0 0 0\nvt 0 0\nf 1/1 1 1\n").is_err());
    }
}
