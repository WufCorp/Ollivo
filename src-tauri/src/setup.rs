//! Мастер первого запуска: проверки ПК, выбор папки, установка VC++ Runtime.

use crate::hardware::{Disk, Hardware};
use serde::Serialize;
use std::path::{Path, PathBuf};

const GIB: u64 = 1 << 30;
/// Меньше — предупреждаем: одна модель занимает 2–10 ГБ.
pub const MIN_FREE: u64 = 20 * GIB;

/// Официальный установщик VC++ 2015–2022 x64 (постоянная ссылка Microsoft).
pub const VC_REDIST_URL: &str = "https://aka.ms/vs/17/release/vc_redist.x64.exe";
/// Эти библиотеки импортирует llama.cpp (проверено по b11081).
const VC_DLLS: &[&str] = &["msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub title: &'static str,
    pub status: Status,
    pub message: String,
    /// Что может сделать программа сама: `"vcredist"` — поставить VC++.
    pub fix: Option<&'static str>,
}

fn system32() -> PathBuf {
    let win = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    Path::new(&win).join("System32")
}

pub fn has_vc_runtime() -> bool {
    let dir = system32();
    VC_DLLS.iter().all(|d| dir.join(d).is_file())
}

pub fn has_vulkan() -> bool {
    system32().join("vulkan-1.dll").is_file()
}

pub fn checks(hw: &Hardware) -> Vec<Check> {
    let gb = |b: u64| format!("{:.1}", b as f64 / GIB as f64).replace('.', ",");
    let mut out = Vec::new();

    out.push(match &hw.gpu {
        Some(g) => Check {
            id: "gpu",
            title: "Видеокарта",
            status: if g.vram_total >= 4 * GIB { Status::Ok } else { Status::Warn },
            message: if g.vram_total >= 4 * GIB {
                format!("{}, {} ГБ", g.name, gb(g.vram_total))
            } else {
                format!("{}, {} ГБ — подойдут только маленькие модели", g.name, gb(g.vram_total))
            },
            fix: None,
        },
        None => Check {
            id: "gpu",
            title: "Видеокарта",
            status: Status::Warn,
            message: "NVIDIA не найдена. Чат будет работать, но медленнее, картинки и видео — вряд ли".into(),
            fix: None,
        },
    });

    if hw.gpu.is_some() {
        let ok = hw.cuda_driver >= 12040;
        out.push(Check {
            id: "driver",
            title: "Драйвер NVIDIA",
            status: if ok { Status::Ok } else { Status::Warn },
            message: if ok {
                format!("{}, свежий", hw.driver)
            } else {
                format!("{} — устарел, обновите через приложение NVIDIA или сайт nvidia.com", hw.driver)
            },
            fix: None,
        });
    }

    out.push(if has_vulkan() {
        Check { id: "vulkan", title: "Vulkan", status: Status::Ok, message: "есть".into(), fix: None }
    } else {
        Check {
            id: "vulkan",
            title: "Vulkan",
            status: Status::Warn,
            message: "нет — обычно ставится с драйвером видеокарты. Чат поставим на CUDA".into(),
            fix: None,
        }
    });

    out.push(if has_vc_runtime() {
        Check { id: "vcredist", title: "Библиотеки Microsoft VC++", status: Status::Ok, message: "есть".into(), fix: None }
    } else {
        Check {
            id: "vcredist",
            title: "Библиотеки Microsoft VC++",
            status: Status::Fail,
            message: "нет — без них движок чата не запустится. Поставим официальный установщик Microsoft".into(),
            fix: Some("vcredist"),
        }
    });

    out.push(Check {
        id: "ram",
        title: "Оперативная память",
        status: if hw.ram_total >= 15 * GIB { Status::Ok } else { Status::Warn },
        message: if hw.ram_total >= 15 * GIB {
            format!("{} ГБ", gb(hw.ram_total))
        } else {
            format!("{} ГБ — большие модели не поместятся, подберём поменьше", gb(hw.ram_total))
        },
        fix: None,
    });

    out
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskChoice {
    pub mount: String,
    /// Куда поставим: `<диск>\Ollivo`.
    pub path: PathBuf,
    pub free: u64,
    pub total: u64,
    pub enough: bool,
    pub recommended: bool,
}

pub fn disk_choices(disks: &[Disk]) -> Vec<DiskChoice> {
    let best = crate::settings::suggest_data_dir(disks);
    disks
        .iter()
        .filter(|d| d.total > 0)
        .map(|d| {
            let path = Path::new(&d.mount).join("Ollivo");
            DiskChoice {
                recommended: path == best,
                enough: d.free >= MIN_FREE,
                mount: d.mount.clone(),
                path,
                free: d.free,
                total: d.total,
            }
        })
        .collect()
}

/// Создаёт папку и проверяет, что в неё можно писать.
pub fn prepare_dir(path: &Path) -> Result<(), String> {
    let s = path.to_string_lossy();
    if crate::hardware::is_risky_path(&s) {
        return Err("в пути не должно быть русских букв и пробелов — иначе часть движков не заработает".into());
    }
    std::fs::create_dir_all(path).map_err(|e| format!("не получилось создать {s}: {e}"))?;
    let probe = path.join(".ollivo-write-test");
    std::fs::write(&probe, b"ok").map_err(|e| format!("в папку {s} нельзя писать: {e}"))?;
    let _ = std::fs::remove_file(probe);
    Ok(())
}

/// Запускает установщик VC++ с запросом прав администратора и ждёт его.
/// Код 0 — установлено, 3010 — установлено, нужна перезагрузка, 1638 — уже есть новее.
pub fn run_vc_redist(exe: &Path) -> Result<(), String> {
    let script = format!(
        "$p = Start-Process -FilePath '{}' -ArgumentList '/install','/quiet','/norestart' -Verb RunAs -Wait -PassThru; exit $p.ExitCode",
        exe.display().to_string().replace('\'', "''")
    );
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()
        .map_err(|e| e.to_string())?;
    match status.code() {
        Some(0 | 3010 | 1638) => Ok(()),
        // Отказ в окне UAC: Start-Process падает, PowerShell возвращает 1.
        Some(1) => Err("установка отменена — Windows не получила разрешения".into()),
        Some(c) => Err(format!("установщик Microsoft завершился с кодом {c}")),
        None => Err("установщик Microsoft прерван".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_choice_marks_best_and_small() {
        let disks = vec![
            Disk { mount: r"C:\".into(), total: 250 * GIB, free: 9 * GIB },
            Disk { mount: r"D:\".into(), total: 2000 * GIB, free: 136 * GIB },
        ];
        let c = disk_choices(&disks);
        assert!(!c[0].enough && !c[0].recommended);
        assert!(c[1].enough && c[1].recommended);
        assert_eq!(c[1].path, PathBuf::from(r"D:\Ollivo"));
    }

    #[test]
    fn prepare_rejects_risky_path() {
        assert!(prepare_dir(Path::new(r"C:\Users\Иван\Ollivo")).is_err());
        let ok = std::env::temp_dir().join(format!("ollivo-setup-{}", std::process::id()));
        if !crate::hardware::is_risky_path(&ok.to_string_lossy()) {
            prepare_dir(&ok).unwrap();
        }
    }

    #[test]
    fn this_pc_has_runtime() {
        // На ПК разработки VC++ и Vulkan есть — проверка не должна ошибаться.
        assert!(has_vc_runtime());
    }
}
