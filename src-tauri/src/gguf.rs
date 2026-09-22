//! Чтение заголовка GGUF: метаданные и описания тензоров. Сами веса не читаются.

use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, ErrorKind, Read};
use std::path::Path;

#[derive(Debug, Clone)]
pub enum Value {
    Int(i128),
    Float(f64),
    Bool(bool),
    Str(String),
    /// Массив: длина и числовые элементы (если массив числовой и небольшой).
    Arr { len: u64, nums: Vec<i128> },
}

impl Value {
    pub fn as_int(&self) -> Option<i128> {
        match self {
            Value::Int(v) => Some(*v),
            // Некоторые модели хранят значение по слоям — берём максимум.
            Value::Arr { nums, .. } => nums.iter().copied().max(),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
}

pub struct Tensor {
    pub name: String,
    pub ggml_type: u32,
    pub elements: u64,
    pub bytes: u64,
}

pub struct Gguf {
    pub version: u32,
    pub kv: HashMap<String, Value>,
    pub tensors: Vec<Tensor>,
}

impl Gguf {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.kv.get(key)
    }
    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }
    pub fn int(&self, key: &str) -> Option<u64> {
        self.get(key).and_then(Value::as_int).map(|v| v as u64)
    }
    /// Параметр архитектуры: `<arch>.<name>`.
    pub fn arch_int(&self, name: &str) -> Option<u64> {
        let arch = self.str("general.architecture")?;
        self.int(&format!("{arch}.{name}"))
    }
}

struct Reader<R: Read> {
    r: R,
}

impl<R: Read> Reader<R> {
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut b = [0u8; N];
        self.r.read_exact(&mut b).map_err(truncated)?;
        Ok(b)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.bytes()?))
    }
    fn string(&mut self) -> Result<String> {
        let len = self.u64()?;
        if len > 64 << 20 {
            bail!("слишком длинная строка ({len} байт) — файл повреждён");
        }
        let mut buf = vec![0u8; len as usize];
        self.r.read_exact(&mut buf).map_err(truncated)?;
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }
    fn skip(&mut self, n: u64) -> Result<()> {
        let copied = std::io::copy(&mut (&mut self.r).take(n), &mut std::io::sink())?;
        if copied < n {
            bail!("файл обрезан: заголовок GGUF не дочитан");
        }
        Ok(())
    }

    fn scalar(&mut self, ty: u32) -> Result<Value> {
        Ok(match ty {
            0 => Value::Int(self.bytes::<1>()?[0] as i128),
            1 => Value::Int(self.bytes::<1>()?[0] as i8 as i128),
            2 => Value::Int(u16::from_le_bytes(self.bytes()?) as i128),
            3 => Value::Int(i16::from_le_bytes(self.bytes()?) as i128),
            4 => Value::Int(self.u32()? as i128),
            5 => Value::Int(i32::from_le_bytes(self.bytes()?) as i128),
            6 => Value::Float(f32::from_le_bytes(self.bytes()?) as f64),
            7 => Value::Bool(self.bytes::<1>()?[0] != 0),
            8 => Value::Str(self.string()?),
            10 => Value::Int(self.u64()? as i128),
            11 => Value::Int(i64::from_le_bytes(self.bytes()?) as i128),
            12 => Value::Float(f64::from_le_bytes(self.bytes()?)),
            _ => bail!("неизвестный тип значения GGUF: {ty}"),
        })
    }

    fn value(&mut self, ty: u32) -> Result<Value> {
        if ty != 9 {
            return self.scalar(ty);
        }
        let elem = self.u32()?;
        let len = self.u64()?;
        let mut nums = Vec::new();
        match elem {
            // Строки (словарь токенизатора и т.п.) не храним — только пропускаем.
            8 => {
                for _ in 0..len {
                    let n = self.u64()?;
                    self.skip(n)?;
                }
            }
            9 => bail!("вложенные массивы в GGUF не поддерживаются"),
            _ => {
                let size = scalar_size(elem)?;
                if len <= 4096 {
                    for _ in 0..len {
                        if let Value::Int(v) = self.scalar(elem)? {
                            nums.push(v);
                        }
                    }
                } else {
                    self.skip(len * size)?;
                }
            }
        }
        Ok(Value::Arr { len, nums })
    }
}

fn scalar_size(ty: u32) -> Result<u64> {
    Ok(match ty {
        0 | 1 | 7 => 1,
        2 | 3 => 2,
        4 | 5 | 6 => 4,
        10 | 11 | 12 => 8,
        _ => bail!("неизвестный тип элемента массива GGUF: {ty}"),
    })
}

fn truncated(e: std::io::Error) -> anyhow::Error {
    if e.kind() == ErrorKind::UnexpectedEof {
        anyhow::anyhow!("файл обрезан: заголовок GGUF не дочитан")
    } else {
        e.into()
    }
}

/// (элементов в блоке, байт на блок) для типов ggml.
pub fn ggml_block(ty: u32) -> Option<(u64, u64)> {
    Some(match ty {
        0 => (1, 4),     // F32
        1 => (1, 2),     // F16
        2 => (32, 18),   // Q4_0
        3 => (32, 20),   // Q4_1
        6 => (32, 22),   // Q5_0
        7 => (32, 24),   // Q5_1
        8 => (32, 34),   // Q8_0
        9 => (32, 36),   // Q8_1
        10 => (256, 84), // Q2_K
        11 => (256, 110),
        12 => (256, 144),
        13 => (256, 176),
        14 => (256, 210),
        15 => (256, 292),
        16 => (256, 66),
        17 => (256, 74),
        18 => (256, 98),
        19 => (256, 50),
        20 => (32, 18),
        21 => (256, 110),
        22 => (256, 82),
        23 => (256, 136),
        24 => (1, 1),
        25 => (1, 2),
        26 => (1, 4),
        27 => (1, 8),
        28 => (1, 8),
        29 => (256, 56),
        30 => (1, 2), // BF16
        34 => (256, 54),
        35 => (256, 66),
        39 => (32, 17), // MXFP4
        _ => return None,
    })
}

pub fn ggml_type_name(ty: u32) -> &'static str {
    match ty {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        6 => "Q5_0",
        7 => "Q5_1",
        8 => "Q8_0",
        10 => "Q2_K",
        11 => "Q3_K",
        12 => "Q4_K",
        13 => "Q5_K",
        14 => "Q6_K",
        16 => "IQ2_XXS",
        17 => "IQ2_XS",
        18 => "IQ3_XXS",
        19 => "IQ1_S",
        20 => "IQ4_NL",
        21 => "IQ3_S",
        22 => "IQ2_S",
        23 => "IQ4_XS",
        29 => "IQ1_M",
        30 => "BF16",
        34 => "TQ1_0",
        35 => "TQ2_0",
        39 => "MXFP4",
        _ => "?",
    }
}

pub fn read(path: &Path) -> Result<Gguf> {
    let f = File::open(path).with_context(|| format!("не открыть {}", path.display()))?;
    let mut r = Reader { r: BufReader::with_capacity(1 << 20, f) };
    if &r.bytes::<4>()? != b"GGUF" {
        bail!("это не GGUF");
    }
    let version = r.u32()?;
    if !(2..=3).contains(&version) {
        bail!("версия GGUF {version} не поддерживается");
    }
    let n_tensors = r.u64()?;
    let n_kv = r.u64()?;
    if n_tensors > 1_000_000 || n_kv > 1_000_000 {
        bail!("заголовок GGUF повреждён");
    }

    let mut kv = HashMap::new();
    for _ in 0..n_kv {
        let key = r.string()?;
        let ty = r.u32()?;
        let v = r.value(ty).with_context(|| format!("ключ {key}"))?;
        kv.insert(key, v);
    }

    let mut tensors = Vec::with_capacity(n_tensors as usize);
    for _ in 0..n_tensors {
        let name = r.string()?;
        let n_dims = r.u32()?;
        let mut elements = 1u64;
        for _ in 0..n_dims {
            elements = elements.saturating_mul(r.u64()?);
        }
        let ggml_type = r.u32()?;
        let _offset = r.u64()?;
        let bytes = match ggml_block(ggml_type) {
            Some((blk, size)) => elements.div_ceil(blk) * size,
            None => 0,
        };
        tensors.push(Tensor { name, ggml_type, elements, bytes });
    }
    Ok(Gguf { version, kv, tensors })
}
