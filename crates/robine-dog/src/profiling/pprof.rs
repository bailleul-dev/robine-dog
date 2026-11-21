//! pprof (`profile.proto`), as the Datadog profilers send it, folded into
//! stacks: `root;caller;leaf` with a value per sample type, by endpoint (the
//! `trace endpoint` label ddtrace puts on samples taken in a request).

use prost::Message;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;

#[derive(Clone, PartialEq, Message)]
pub struct Profile {
    #[prost(message, repeated, tag = "1")]
    pub sample_type: Vec<ValueType>,
    #[prost(message, repeated, tag = "2")]
    pub sample: Vec<Sample>,
    #[prost(message, repeated, tag = "4")]
    pub location: Vec<Location>,
    #[prost(message, repeated, tag = "5")]
    pub function: Vec<Function>,
    #[prost(string, repeated, tag = "6")]
    pub string_table: Vec<String>,
    #[prost(int64, tag = "9")]
    pub time_nanos: i64,
    #[prost(int64, tag = "10")]
    pub duration_nanos: i64,
    #[prost(message, optional, tag = "11")]
    pub period_type: Option<ValueType>,
    #[prost(int64, tag = "12")]
    pub period: i64,
}

#[derive(Clone, PartialEq, Message)]
pub struct ValueType {
    #[prost(int64, tag = "1")]
    pub r#type: i64,
    #[prost(int64, tag = "2")]
    pub unit: i64,
}

#[derive(Clone, PartialEq, Message)]
pub struct Sample {
    #[prost(uint64, repeated, tag = "1")]
    pub location_id: Vec<u64>,
    #[prost(int64, repeated, tag = "2")]
    pub value: Vec<i64>,
    #[prost(message, repeated, tag = "3")]
    pub label: Vec<Label>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Label {
    #[prost(int64, tag = "1")]
    pub key: i64,
    #[prost(int64, tag = "2")]
    pub str: i64,
    #[prost(int64, tag = "3")]
    pub num: i64,
}

#[derive(Clone, PartialEq, Message)]
pub struct Location {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(uint64, tag = "3")]
    pub address: u64,
    #[prost(message, repeated, tag = "4")]
    pub line: Vec<Line>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Line {
    #[prost(uint64, tag = "1")]
    pub function_id: u64,
    #[prost(int64, tag = "2")]
    pub line: i64,
}

#[derive(Clone, PartialEq, Message)]
pub struct Function {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(int64, tag = "2")]
    pub name: i64,
    #[prost(int64, tag = "3")]
    pub system_name: i64,
    #[prost(int64, tag = "4")]
    pub filename: i64,
}

/// Undoes gzip or zstd, which profilers apply to pprof files.
pub fn decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::MultiGzDecoder::new(data)
            .read_to_end(&mut out)
            .map_err(|e| format!("gzip: {e}"))?;
        Ok(out)
    } else if data.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        zstd::decode_all(data).map_err(|e| format!("zstd: {e}"))
    } else if data.starts_with(&[0x04, 0x22, 0x4d, 0x18]) {
        Err("lz4 profiles are not supported: use gzip or zstd".into())
    } else {
        Ok(data.to_vec())
    }
}

/// Folded stacks of one sample type: `(endpoint, stack) → value`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Folded {
    /// The type and its unit, e.g. `cpu` / `nanoseconds`.
    pub unit: String,
    /// `[endpoint, stack, value]`, largest first.
    pub stacks: Vec<(String, String, i64)>,
}

/// Most stacks kept per sample type: the rest are summed into `[other]`.
pub const MAX_STACKS: usize = 5000;

pub fn decode(data: &[u8]) -> Result<Profile, String> {
    let raw = decompress(data)?;
    Profile::decode(raw.as_slice()).map_err(|e| format!("not a pprof profile: {e}"))
}

/// Folds a profile: one entry per sample type (`cpu-time`, `wall-time`,
/// `alloc-space`…).
pub fn fold(p: &Profile) -> HashMap<String, Folded> {
    let s = |i: i64| {
        p.string_table
            .get(i as usize)
            .map(String::as_str)
            .unwrap_or("")
    };
    let functions: HashMap<u64, String> = p
        .function
        .iter()
        .map(|f| {
            let name = match s(f.name) {
                "" => s(f.system_name),
                n => n,
            };
            let file = s(f.filename);
            let label = if file.is_empty() || name.contains(file) {
                name.to_string()
            } else {
                format!("{name} ({})", file.rsplit('/').next().unwrap_or(file))
            };
            (f.id, label.replace(';', ","))
        })
        .collect();
    // A location's frames, caller first (pprof lists inlined frames leaf first).
    let frames: HashMap<u64, Vec<String>> = p
        .location
        .iter()
        .map(|l| {
            let mut names: Vec<String> = l
                .line
                .iter()
                .map(|line| {
                    functions
                        .get(&line.function_id)
                        .cloned()
                        .unwrap_or_else(|| "?".into())
                })
                .collect();
            if names.is_empty() {
                names.push(format!("0x{:x}", l.address));
            }
            names.reverse();
            (l.id, names)
        })
        .collect();
    let endpoint_key = p
        .string_table
        .iter()
        .position(|x| x == "trace endpoint")
        .map(|i| i as i64);
    let types: Vec<(String, String)> = p
        .sample_type
        .iter()
        .map(|t| (s(t.r#type).to_string(), s(t.unit).to_string()))
        .collect();
    let mut acc: Vec<HashMap<(String, String), i64>> = vec![HashMap::new(); types.len()];
    for sample in &p.sample {
        if sample.value.iter().all(|v| *v == 0) {
            continue;
        }
        // Locations are leaf first: the stack reads from the root.
        let stack = sample
            .location_id
            .iter()
            .rev()
            .flat_map(|id| frames.get(id).cloned().unwrap_or_else(|| vec!["?".into()]))
            .collect::<Vec<_>>()
            .join(";");
        let endpoint = endpoint_key
            .and_then(|k| sample.label.iter().find(|l| l.key == k))
            .map(|l| s(l.str).to_string())
            .unwrap_or_default();
        for (i, v) in sample.value.iter().enumerate() {
            if *v != 0 && i < acc.len() {
                *acc[i].entry((endpoint.clone(), stack.clone())).or_default() += v;
            }
        }
    }
    types
        .into_iter()
        .zip(acc)
        .filter(|(_, m)| !m.is_empty())
        .map(|((name, unit), m)| {
            let mut stacks: Vec<(String, String, i64)> =
                m.into_iter().map(|((e, s), v)| (e, s, v)).collect();
            stacks.sort_by_key(|a| std::cmp::Reverse(a.2));
            if stacks.len() > MAX_STACKS {
                let rest: i64 = stacks[MAX_STACKS..].iter().map(|x| x.2).sum();
                stacks.truncate(MAX_STACKS);
                stacks.push((String::new(), "[other]".into(), rest));
            }
            (name, Folded { unit, stacks })
        })
        .collect()
}

/// Is a type a snapshot (heap in use), averaged over profiles rather than
/// summed?
pub fn is_snapshot(sample_type: &str) -> bool {
    sample_type.contains("inuse") || sample_type.starts_with("heap") || sample_type.contains("live")
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A small profile: `main → handle → query` and `main → render`, CPU and
    /// allocations, one sample in an endpoint.
    pub fn sample_profile() -> Profile {
        let strings: Vec<String> = [
            "",
            "cpu-time",
            "nanoseconds",
            "alloc-space",
            "bytes",
            "main",
            "app.py",
            "handle",
            "query",
            "db.py",
            "render",
            "trace endpoint",
            "GET /products",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let f = |id: u64, name: i64, file: i64| Function {
            id,
            name,
            system_name: 0,
            filename: file,
        };
        let loc = |id: u64, func: u64| Location {
            id,
            address: 0,
            line: vec![Line {
                function_id: func,
                line: 1,
            }],
        };
        Profile {
            sample_type: vec![
                ValueType { r#type: 1, unit: 2 },
                ValueType { r#type: 3, unit: 4 },
            ],
            sample: vec![
                Sample {
                    location_id: vec![3, 2, 1],
                    value: vec![300, 1000],
                    label: vec![Label {
                        key: 11,
                        str: 12,
                        num: 0,
                    }],
                },
                Sample {
                    location_id: vec![4, 1],
                    value: vec![100, 0],
                    label: vec![],
                },
            ],
            location: vec![loc(1, 1), loc(2, 2), loc(3, 3), loc(4, 4)],
            function: vec![f(1, 5, 6), f(2, 7, 6), f(3, 8, 9), f(4, 10, 6)],
            string_table: strings,
            time_nanos: 1_700_000_000_000_000_000,
            duration_nanos: 60_000_000_000,
            period_type: None,
            period: 0,
        }
    }

    #[test]
    fn folds_stacks_by_type_and_endpoint() {
        let p = sample_profile();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &p.encode_to_vec()).unwrap();
        let decoded = decode(&gz.finish().unwrap()).unwrap();
        assert_eq!(decoded, p);
        let f = fold(&decoded);
        let cpu = &f["cpu-time"];
        assert_eq!(cpu.unit, "nanoseconds");
        assert_eq!(
            cpu.stacks[0],
            (
                "GET /products".into(),
                "main (app.py);handle (app.py);query (db.py)".into(),
                300
            )
        );
        assert_eq!(
            cpu.stacks[1],
            ("".into(), "main (app.py);render (app.py)".into(), 100)
        );
        assert_eq!(f["alloc-space"].stacks.len(), 1);
        let z = zstd::encode_all(p.encode_to_vec().as_slice(), 1).unwrap();
        assert_eq!(decode(&z).unwrap(), p);
        assert!(is_snapshot("inuse-space") && !is_snapshot("cpu-time"));
    }
}
