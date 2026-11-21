//! Coverage report (offs-style `report.txt`).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use crate::scan::Scan;

fn size_str(n: u64) -> String {
    const UNITS: [&str; 3] = ["KiB", "MiB", "GiB"];
    let unit = ((n.max(1).ilog2() / 10).min(3) as usize).max(1);
    let unit_name = UNITS[unit - 1];
    let value = n as f64 / (1u64 << (10 * unit)) as f64;
    format!("{value:.1} {unit_name}")
}

/// Build the coverage report text (offs-style).
pub fn render(name: &str, data_len: usize, scan: &Scan) -> String {
    let mut s = String::new();
    s.push_str(&format!("{name}  {data_len} B  {} archives\n", scan.archives.len()));

    s.push_str("\n    archive        size   files   found\n");
    for (i, a) in scan.archives.iter().enumerate() {
        let found = scan.owned_streams(i).len();
        s.push_str(&format!(
            "{:>#11x}  {:>10}  {:>6}  {:>6}\n",
            a.header.at,
            size_str(a.extent().len() as u64),
            a.header.files,
            found
        ));
    }

    let mut types: BTreeMap<(&'static str, &'static str), usize> = BTreeMap::new();
    for (i, _a) in scan.archives.iter().enumerate() {
        for s in scan.owned_streams(i) {
            *types.entry(s.kind.label()).or_default() += 1;
        }
    }
    s.push_str("\n  type   files  kind\n");
    for ((ext, name), files) in types {
        s.push_str(&format!("{ext:>6}  {files:>6}  {name}\n"));
    }

    let mut structure = 0u64;
    let mut zlib = 0u64;
    let mut media = 0u64;
    for a in &scan.archives {
        for r in a.structure() {
            structure += r.len() as u64;
        }
    }
    for s in &scan.streams {
        if s.kind.is_media() {
            media += s.stored_len as u64;
        } else {
            zlib += s.stored_len as u64;
        }
    }
    s.push_str("\n    coverage              bytes\n");
    s.push_str(&format!("{:>12}  {:>14}\n", "structure", structure));
    s.push_str(&format!("{:>12}  {:>14}\n", "zlib", zlib));
    s.push_str(&format!("{:>12}  {:>14}\n", "media", media));
    s.push_str(&format!(
        "{:>12}  {:>14}\n",
        "unexplained",
        (data_len as u64).saturating_sub(structure + zlib + media)
    ));

    s
}

pub fn write_report(path: &Path, name: &str, data_len: usize, scan: &Scan) -> io::Result<()> {
    fs::write(path, render(name, data_len, scan))
}
