//! Железо ПК: видеокарта (NVML), память, диски. Отсюда мастер первого запуска
//! и манифест выбирают сборки движков.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Gpu {
    pub name: String,
    pub vram_total: u64,
    pub vram_free: u64,
    /// Compute capability, например (6, 1) у GTX 10xx.
    pub cc: (u32, u32),
    /// Пропускная способность видеопамяти, байт/с (оценка).
    pub vram_bw: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Disk {
    /// Точка монтирования, например `D:\`.
    pub mount: String,
    pub total: u64,
    pub free: u64,
}

/// Какую сборку движков ставить. Решение из фазы 0: CUDA 13 не работает
/// на Maxwell/Pascal/Volta (CC < 7.5), для них — CUDA 12 и torch cu126.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Build {
    Cuda13,
    Cuda12,
    Vulkan,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hardware {
    pub gpu: Option<Gpu>,
    pub driver: String,
    /// Версия CUDA, которую поддерживает драйвер, например 13000.
    pub cuda_driver: i32,
    /// Сборка CUDA для llama.cpp / whisper.cpp / torch; `Vulkan`, если CUDA нет.
    pub cuda_build: Build,
    pub ram_total: u64,
    pub ram_avail: u64,
    pub disks: Vec<Disk>,
    /// Профиль пользователя с кириллицей или пробелом — программу и модели
    /// ставим не в него, а в `<диск>:\Ollivo`.
    pub profile_risky: bool,
}

pub fn detect() -> Hardware {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let (gpu, driver, cuda_driver) = detect_gpu();
    Hardware {
        cuda_build: cuda_build(gpu.as_ref().map(|g| g.cc), cuda_driver),
        gpu,
        driver,
        cuda_driver,
        ram_total: sys.total_memory(),
        ram_avail: sys.available_memory(),
        disks: detect_disks(),
        profile_risky: std::env::var("USERPROFILE").is_ok_and(|p| is_risky_path(&p)),
    }
}

fn detect_gpu() -> (Option<Gpu>, String, i32) {
    let Ok(nvml) = nvml_wrapper::Nvml::init() else {
        return (None, String::new(), 0);
    };
    let driver = nvml.sys_driver_version().unwrap_or_default();
    let cuda_driver = nvml.sys_cuda_driver_version().unwrap_or(0);
    let Ok(dev) = nvml.device_by_index(0) else {
        return (None, driver, cuda_driver);
    };
    let (vram_total, vram_free) = dev.memory_info().map(|m| (m.total, m.free)).unwrap_or((0, 0));
    let cc = dev
        .cuda_compute_capability()
        .map(|c| (c.major as u32, c.minor as u32))
        .unwrap_or((0, 0));
    let bus = dev.memory_bus_width().unwrap_or(0) as u64;
    let clk = dev
        .max_clock_info(nvml_wrapper::enum_wrappers::device::Clock::Memory)
        .unwrap_or(0) as u64;
    let gpu = Gpu {
        name: dev.name().unwrap_or_default(),
        vram_total,
        vram_free,
        cc,
        vram_bw: clk * 1_000_000 * 2 * bus / 8,
    };
    (Some(gpu), driver, cuda_driver)
}

fn cuda_build(cc: Option<(u32, u32)>, cuda_driver: i32) -> Build {
    match cc {
        Some(cc) if cc >= (7, 5) && cuda_driver >= 13000 => Build::Cuda13,
        Some(cc) if cc >= (5, 0) && cuda_driver >= 12040 => Build::Cuda12,
        _ => Build::Vulkan,
    }
}

fn detect_disks() -> Vec<Disk> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut out: Vec<Disk> = disks
        .list()
        .iter()
        .filter(|d| !d.is_removable())
        .map(|d| Disk {
            mount: d.mount_point().to_string_lossy().into_owned(),
            total: d.total_space(),
            free: d.available_space(),
        })
        .collect();
    out.sort_by(|a, b| a.mount.cmp(&b.mount));
    out
}

/// Путь, в котором встроенный Python и часть движков могут сломаться:
/// не-ASCII символы (кириллица в имени пользователя) или пробелы.
pub fn is_risky_path(path: &str) -> bool {
    !path.is_ascii() || path.contains(' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_by_compute_capability() {
        // GTX 1080 с драйвером под CUDA 13 — всё равно CUDA 12.
        assert_eq!(cuda_build(Some((6, 1)), 13000), Build::Cuda12);
        assert_eq!(cuda_build(Some((8, 6)), 13000), Build::Cuda13);
        // Старый драйвер у RTX — CUDA 12.
        assert_eq!(cuda_build(Some((8, 6)), 12080), Build::Cuda12);
        assert_eq!(cuda_build(Some((8, 6)), 11080), Build::Vulkan);
        assert_eq!(cuda_build(None, 0), Build::Vulkan);
    }

    #[test]
    fn risky_paths() {
        assert!(is_risky_path(r"C:\Users\Иван\AppData"));
        assert!(is_risky_path(r"C:\Program Files\Ollivo"));
        assert!(!is_risky_path(r"D:\Ollivo"));
    }
}
