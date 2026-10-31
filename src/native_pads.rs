//! Leitura direta de controles pelo joydev (`/dev/input/js*`).
//!
//! O Chromium ignora controles da Nintendo e da Sony no caminho genérico do Linux e
//! tenta falar com eles via HID, o que falha quando o driver do kernel
//! (`hid-nintendo`, `hid-playstation`) já é dono do dispositivo. Aqui lemos esses
//! controles por conta própria, no processo de renderização, e o tradutor de input
//! os mescla em `navigator.getGamepads()`. Junto vão os códigos evdev de cada
//! botão/eixo, para o tradutor mapear pelo significado e não pelo índice.

use serde::Serialize;
use std::{
    collections::HashSet,
    fs::{self, File},
    io::Read,
    os::fd::AsRawFd,
    path::Path,
    sync::{Mutex, Once},
    thread,
    time::Duration,
};

/// Vendors que o Chromium não expõe pelo joydev.
const VENDORS: &[&str] = &[
    "057e", // Nintendo
    "054c", // Sony
];

const JS_EVENT_BUTTON: u8 = 0x01;
const JS_EVENT_AXIS: u8 = 0x02;
const JS_EVENT_INIT: u8 = 0x80;

// ioctls do joydev (`linux/joystick.h`).
const JSIOCGAXES: u64 = 0x8001_6a11;
const JSIOCGBUTTONS: u64 = 0x8001_6a12;
const JSIOCGAXMAP: u64 = 0x8040_6a32; // __u8[ABS_CNT]
const JSIOCGBTNMAP: u64 = 0x8400_6a34; // __u16[KEY_MAX - BTN_MISC + 1]

unsafe extern "C" {
    fn ioctl(fd: i32, request: u64, ...) -> i32;
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativePad {
    /// Ex.: `/dev/input/js0`; identifica o controle entre leituras.
    pub device: String,
    /// No formato do Chromium: `Nome (Vendor: 057e Product: 2009)`.
    pub id: String,
    pub connected: bool,
    /// Incrementa a cada evento; o JS usa para atualizar o `timestamp`.
    pub seq: u64,
    pub buttons: Vec<f32>,
    pub axes: Vec<f32>,
    /// Código evdev de cada botão (ex.: `0x130` = `BTN_SOUTH`).
    pub button_codes: Vec<u16>,
    /// Código evdev de cada eixo (ex.: `0x05` = `ABS_RZ`).
    pub axis_codes: Vec<u8>,
}

/// Códigos evdev dos botões e eixos do joydev, na ordem dos índices.
fn joydev_maps(file: &File) -> (Vec<u16>, Vec<u8>) {
    let fd = file.as_raw_fd();
    let (mut buttons, mut axes) = (0u8, 0u8);
    let mut button_map = [0u16; 512];
    let mut axis_map = [0u8; 64];
    // SAFETY: cada ioctl escreve no máximo o tamanho codificado na própria requisição.
    let ok = unsafe {
        ioctl(fd, JSIOCGBUTTONS, &mut buttons) >= 0
            && ioctl(fd, JSIOCGAXES, &mut axes) >= 0
            && ioctl(fd, JSIOCGBTNMAP, button_map.as_mut_ptr()) >= 0
            && ioctl(fd, JSIOCGAXMAP, axis_map.as_mut_ptr()) >= 0
    };
    if !ok {
        return (Vec::new(), Vec::new());
    }
    (
        button_map[..buttons as usize].to_vec(),
        axis_map[..axes as usize].to_vec(),
    )
}

static PADS: Mutex<Vec<NativePad>> = Mutex::new(Vec::new());
static START: Once = Once::new();

/// Estado atual dos controles (inicia a leitura na primeira chamada).
pub fn snapshot() -> Vec<NativePad> {
    START.call_once(|| {
        thread::Builder::new()
            .name("avenger-pads".into())
            .spawn(scan_loop)
            .expect("falha ao criar o thread de controles");
    });
    PADS.lock().map(|pads| pads.clone()).unwrap_or_default()
}

fn read_sysfs(js: &Path, file: &str) -> Option<String> {
    fs::read_to_string(js.join("device").join(file))
        .ok()
        .map(|value| value.trim().to_lowercase())
}

/// Procura novos controles a cada segundo.
fn scan_loop() {
    let mut open = HashSet::new();
    loop {
        if let Ok(entries) = fs::read_dir("/sys/class/input") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.starts_with("js") || open.contains(&name) {
                    continue;
                }
                let sys = entry.path();
                let Some(vendor) = read_sysfs(&sys, "id/vendor") else {
                    continue;
                };
                if !VENDORS.contains(&vendor.as_str()) {
                    continue;
                }
                let product = read_sysfs(&sys, "id/product").unwrap_or_default();
                let label = fs::read_to_string(sys.join("device/name"))
                    .map(|n| n.trim().to_string())
                    .unwrap_or_else(|_| "Controle".into());
                let device = format!("/dev/input/{name}");
                let Ok(file) = File::open(&device) else {
                    continue;
                };

                let (button_codes, axis_codes) = joydev_maps(&file);

                open.insert(name.clone());
                if let Ok(mut pads) = PADS.lock() {
                    pads.retain(|pad| pad.device != device);
                    pads.push(NativePad {
                        device: device.clone(),
                        id: format!("{label} (Vendor: {vendor} Product: {product})"),
                        connected: true,
                        seq: 0,
                        buttons: vec![0.0; button_codes.len()],
                        axes: vec![0.0; axis_codes.len()],
                        button_codes,
                        axis_codes,
                    });
                }
                thread::spawn(move || read_loop(file, device));
            }
        }
        // Libera nomes de controles desconectados para que voltem a ser detectados.
        if let Ok(pads) = PADS.lock() {
            open.retain(|name| {
                pads.iter()
                    .any(|pad| pad.connected && pad.device.ends_with(&format!("/{name}")))
            });
        }
        thread::sleep(Duration::from_secs(1));
    }
}

/// Lê eventos do joydev até o controle desconectar.
fn read_loop(mut file: File, device: String) {
    // struct js_event { u32 time; i16 value; u8 type; u8 number; }
    let mut event = [0u8; 8];
    while file.read_exact(&mut event).is_ok() {
        let value = i16::from_ne_bytes([event[4], event[5]]);
        let kind = event[6] & !JS_EVENT_INIT;
        let number = event[7] as usize;

        let Ok(mut pads) = PADS.lock() else {
            return;
        };
        let Some(pad) = pads.iter_mut().find(|pad| pad.device == device) else {
            return;
        };
        let list = match kind {
            JS_EVENT_BUTTON => &mut pad.buttons,
            JS_EVENT_AXIS => &mut pad.axes,
            _ => continue,
        };
        if list.len() <= number {
            list.resize(number + 1, 0.0);
        }
        list[number] = match kind {
            JS_EVENT_BUTTON => value as f32,
            _ => (value as f32 / 32767.0).clamp(-1.0, 1.0),
        };
        pad.seq += 1;
    }

    if let Ok(mut pads) = PADS.lock() {
        pads.retain(|pad| pad.device != device);
    }
}
