//! Библиотеки VC++ рядом с движком.
//!
//! llama.cpp и whisper.cpp импортируют `msvcp140`, `vcruntime140`, `vcruntime140_1`.
//! На чистой Windows их нет, а официальный установщик Microsoft просит права
//! администратора — новичок пугается окна UAC или отказывает. Microsoft разрешает класть
//! эти файлы рядом с программой, и Windows ищет DLL сначала в папке exe, потом в System32.
//! Заодно это лечит устаревший VC++ в системе: сборки на свежем компиляторе падают
//! со старым `msvcp140.dll`.
//!
//! Но своя копия рядом выигрывает у системной всегда — и у более новой тоже, а движок,
//! собранный свежее нашей копии, с ней может не заработать. Поэтому копию кладём, только
//! когда в системе библиотек нет или они старее, а если система обновилась — убираем.
//!
//! Файлы берутся при сборке из Redist Visual Studio (`build.rs`), ~0,7 МБ.

use std::path::Path;

const FILES: &[(&str, &[u8])] = &[
    ("msvcp140.dll", include_bytes!(concat!(env!("OUT_DIR"), "/vcrt/msvcp140.dll"))),
    ("vcruntime140.dll", include_bytes!(concat!(env!("OUT_DIR"), "/vcrt/vcruntime140.dll"))),
    ("vcruntime140_1.dll", include_bytes!(concat!(env!("OUT_DIR"), "/vcrt/vcruntime140_1.dll"))),
];

/// Какие из библиотек положили мы (а не архив движка): только их можно убрать.
pub(crate) const MARKER: &str = "ollivo.vcrt";

/// Готовит библиотеки для движка перед запуском. Зовётся перед каждым запуском — так
/// библиотеки появятся и у движков, поставленных прежними версиями Ollivo, вернутся,
/// если их удалили, и уйдут, если в системе появились новее.
/// Ошибка — только когда положить не вышло, а в системе библиотек нет: запуск всё равно
/// упал бы с непонятным кодом, лучше сказать сразу.
pub fn prepare(exe: &Path) -> Result<(), String> {
    let Some(dir) = exe.parent() else { return Ok(()) };
    match ensure_in(dir, &crate::setup::system32()) {
        Err(e) if !crate::setup::has_vc_runtime() => {
            Err(format!("не удалось положить библиотеки Microsoft VC++ рядом с движком: {e}"))
        }
        _ => Ok(()),
    }
}

pub(crate) fn ensure_in(dir: &Path, system: &Path) -> std::io::Result<()> {
    let marker = dir.join(MARKER);
    let ours: Vec<String> = std::fs::read_to_string(&marker)
        .map(|s| s.lines().map(str::to_owned).collect())
        .unwrap_or_default();

    if system_is_enough(system) {
        // Движок запущен — файл занят; что не удалилось, уберём в следующий раз.
        let left: Vec<String> = ours.into_iter().filter(|n| std::fs::remove_file(dir.join(n)).is_err() && dir.join(n).exists()).collect();
        if left.is_empty() {
            let _ = std::fs::remove_file(&marker);
        } else {
            std::fs::write(&marker, left.join("
"))?;
        }
        return Ok(());
    }

    let mut placed = ours;
    for (name, data) in FILES {
        let dest = dir.join(name);
        // Чужие не трогаем: если библиотеки привёз архив движка, они в списке для «Починить».
        if dest.is_file() {
            continue;
        }
        // Через временный файл: оборванная запись не должна оставить половину DLL под настоящим именем.
        let tmp = dir.join(format!("{name}.part"));
        std::fs::write(&tmp, data)?;
        std::fs::rename(&tmp, &dest)?;
        if !placed.iter().any(|p| p == name) {
            placed.push(name.to_string());
        }
    }
    if !placed.is_empty() {
        std::fs::write(&marker, placed.join("\n"))?;
    }
    Ok(())
}

/// В системе все три библиотеки и каждая не старее нашей.
fn system_is_enough(system: &Path) -> bool {
    FILES.iter().all(|(name, data)| {
        let sys = std::fs::read(system.join(name)).ok().and_then(|b| file_version(&b));
        matches!((sys, file_version(data)), (Some(s), Some(o)) if s >= o)
    })
}

/// Версия файла из `VS_FIXEDFILEINFO` (сигнатура `FEEF04BD`): старшая и младшая половины
/// подряд, поэтому сравниваются как одно число. Разбирать ресурсы целиком ради этого незачем.
fn file_version(bytes: &[u8]) -> Option<u64> {
    const SIG: [u8; 4] = [0xBD, 0x04, 0xEF, 0xFE];
    let at = bytes.windows(4).position(|w| w == SIG)?;
    let u32_at = |i: usize| Some(u32::from_le_bytes(bytes.get(i..i + 4)?.try_into().ok()?) as u64);
    Some(u32_at(at + 8)? << 32 | u32_at(at + 12)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("Иван Петров vcrt-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Поддельная DLL с версией `a.b.c.d` — ровно то, что читает `file_version`.
    fn fake_dll(v: [u16; 4]) -> Vec<u8> {
        let mut b = b"MZ padding".to_vec();
        b.extend([0xBD, 0x04, 0xEF, 0xFE, 0, 0, 1, 0]);
        b.extend(((v[0] as u32) << 16 | v[1] as u32).to_le_bytes());
        b.extend(((v[2] as u32) << 16 | v[3] as u32).to_le_bytes());
        b
    }

    fn system_with(v: [u16; 4]) -> PathBuf {
        let sys = tmp(&format!("sys{}", v[1]));
        for (name, _) in FILES {
            std::fs::write(sys.join(name), fake_dll(v)).unwrap();
        }
        sys
    }

    #[test]
    fn embedded_files_are_real_dlls() {
        for (name, data) in FILES {
            assert_eq!(&data[..2], b"MZ", "{name} — не exe/dll");
            let v = file_version(data).unwrap_or_else(|| panic!("{name}: нет версии"));
            assert!(v >> 48 == 14, "{name}: версия {:x} — не VC++ 2015–2022", v);
        }
    }

    #[test]
    fn version_is_read_and_compared() {
        assert_eq!(file_version(&fake_dll([14, 44, 35211, 0])), Some(14 << 48 | 44 << 32 | 35211 << 16));
        assert!(file_version(&fake_dll([14, 51, 1, 0])) > file_version(&fake_dll([14, 44, 35211, 0])));
        assert_eq!(file_version(b"MZ no version"), None);
    }

    /// Чистая Windows: кладём все три, чужую копию из архива не трогаем, повтор ничего не ломает.
    #[test]
    fn clean_windows_gets_copies_beside_engine() {
        let (dir, sys) = (tmp("clean"), tmp("sys-empty"));
        std::fs::write(dir.join("msvcp140.dll"), b"from archive").unwrap();
        ensure_in(&dir, &sys).unwrap();
        assert_eq!(std::fs::read(dir.join("msvcp140.dll")).unwrap(), b"from archive");
        assert_eq!(std::fs::read(dir.join("vcruntime140.dll")).unwrap().len(), FILES[1].1.len());
        assert!(dir.join("vcruntime140_1.dll").is_file());
        assert!(!dir.join("vcruntime140.dll.part").exists());
        ensure_in(&dir, &sys).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(MARKER)).unwrap(), "vcruntime140.dll\nvcruntime140_1.dll");
    }

    /// Старый VC++ в системе — своя копия нужна; новее нашей — своя не нужна и убирается.
    #[test]
    fn copies_follow_system_version() {
        let dir = tmp("follow");
        ensure_in(&dir, &system_with([14, 0, 24215, 1])).unwrap();
        assert!(dir.join("msvcp140.dll").is_file(), "старый VC++ — кладём свою");

        ensure_in(&dir, &system_with([99, 0, 0, 0])).unwrap();
        for (name, _) in FILES {
            assert!(!dir.join(name).exists(), "{name}: в системе новее — своя копия лишняя");
        }
        assert!(!dir.join(MARKER).exists());
    }
}
