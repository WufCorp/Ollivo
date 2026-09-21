//! Чтение заголовка safetensors: JSON с именами, типами и формами тензоров.

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub struct Tensor {
    pub name: String,
    pub dtype: String,
    pub shape: Vec<u64>,
    pub bytes: u64,
}

pub struct Safetensors {
    pub metadata: Map<String, Value>,
    pub tensors: Vec<Tensor>,
}

impl Safetensors {
    pub fn has_prefix(&self, p: &str) -> bool {
        self.tensors.iter().any(|t| t.name.starts_with(p))
    }
    pub fn has(&self, part: &str) -> bool {
        self.tensors.iter().any(|t| t.name.contains(part))
    }
    pub fn find(&self, part: &str) -> Option<&Tensor> {
        self.tensors.iter().find(|t| t.name.contains(part))
    }
    pub fn meta(&self, key: &str) -> Option<&str> {
        self.metadata.get(key).and_then(Value::as_str)
    }
    pub fn bytes_where(&self, f: impl Fn(&str) -> bool) -> u64 {
        self.tensors.iter().filter(|t| f(&t.name)).map(|t| t.bytes).sum()
    }
}

pub fn dtype_size(dtype: &str) -> u64 {
    match dtype {
        "F64" | "I64" | "U64" => 8,
        "F32" | "I32" | "U32" => 4,
        "F16" | "BF16" | "I16" | "U16" => 2,
        _ => 1, // I8, U8, BOOL, F8_E4M3, F8_E5M2
    }
}

pub fn read(path: &Path) -> Result<Safetensors> {
    let mut f = File::open(path).with_context(|| format!("не открыть {}", path.display()))?;
    let mut len = [0u8; 8];
    f.read_exact(&mut len).context("файл слишком короткий")?;
    let len = u64::from_le_bytes(len);
    if len < 2 || len > 100 << 20 {
        bail!("это не safetensors (длина заголовка {len})");
    }
    let mut buf = vec![0u8; len as usize];
    f.read_exact(&mut buf).context("файл обрезан: заголовок safetensors не дочитан")?;
    let json: Map<String, Value> =
        serde_json::from_slice(&buf).context("заголовок safetensors — не JSON")?;

    let mut metadata = Map::new();
    let mut tensors = Vec::new();
    for (name, v) in json {
        if name == "__metadata__" {
            if let Value::Object(m) = v {
                metadata = m;
            }
            continue;
        }
        let dtype = v["dtype"].as_str().unwrap_or("?").to_string();
        let shape: Vec<u64> = v["shape"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_u64).collect())
            .unwrap_or_default();
        let bytes = match v["data_offsets"].as_array() {
            Some(o) if o.len() == 2 => o[1].as_u64().unwrap_or(0) - o[0].as_u64().unwrap_or(0),
            _ => shape.iter().product::<u64>() * dtype_size(&dtype),
        };
        tensors.push(Tensor { name, dtype, shape, bytes });
    }
    Ok(Safetensors { metadata, tensors })
}
