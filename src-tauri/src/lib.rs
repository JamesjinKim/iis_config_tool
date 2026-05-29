//! IIS3DWB 진동센서 WiFi 설정 툴 — Rust 백엔드
//!
//! 검증된 "NVS 직접 주입" 방식을 사용합니다:
//!   1. 사용자 WiFi 입력 → NVS 바이너리 생성 (nvs 모듈, ESP-IDF 도구 불필요)
//!   2. espflash 로 NVS 파티션(0x9000)에 주입
//!   3. 디바이스 재부팅 → 저장된 WiFi로 자동 연결 → 부팅 로그(TX)로 IP 확인

use std::io::Read;
use std::time::{Duration, Instant};

use serde::Serialize;
use serialport::SerialPort;

use espflash::connection::{Connection, ResetAfterOperation, ResetBeforeOperation};
use espflash::flasher::Flasher;
use espflash::target::ProgressCallbacks;

mod nvs;

/// NVS 파티션 오프셋 (펌웨어 partition table 과 일치)
const NVS_OFFSET: u32 = 0x9000;
/// NVS 파티션 크기 (24KB)
const NVS_SIZE: usize = 0x6000;
/// NVS 네임스페이스 (펌웨어 config_manager.c 와 일치)
const NVS_NAMESPACE: &str = "devcfg";
/// 통신 속도
const BAUD_RATE: u32 = 115_200;

// ====================================================================
// 제품 / 펌웨어 버전 정보  ★ 회사에서 제품별 GUI 빌드 시 이 블록만 수정 ★
// ====================================================================
// 이 GUI에 번들된 펌웨어가 어떤 제품·버전인지 식별합니다.
// 사용자는 "자기 제품에 맞는 GUI"인지 이 정보로 확인합니다.
/// 제품명 (펌웨어 serial_protocol.c 의 PRODUCT_NAME 과 일치 권장)
const PRODUCT_NAME: &str = "IIS3DWB 진동센서";
/// 제품 모델 코드
const PRODUCT_MODEL: &str = "IIS3DWB-VIB-SENSOR";
/// 번들된 펌웨어 버전 (제품 릴리스 버전)
const FIRMWARE_VERSION: &str = "1.0.0";
/// 펌웨어 빌드에 사용한 ESP-IDF 버전
const ESP_IDF_VERSION: &str = "v5.4.3";
/// 대상 칩
const TARGET_CHIP: &str = "ESP32-S3";
// ====================================================================

/// 툴에 번들된 펌웨어 바이너리 (빌드 시 포함). 각 오프셋은 partition table 기준.
const FW_BOOTLOADER: &[u8] = include_bytes!("../firmware/bootloader.bin");
const FW_PARTITION: &[u8] = include_bytes!("../firmware/partition-table.bin");
const FW_APP: &[u8] = include_bytes!("../firmware/app.bin");
const FW_BOOTLOADER_OFFSET: u32 = 0x0;
const FW_PARTITION_OFFSET: u32 = 0x8000;
const FW_APP_OFFSET: u32 = 0x10000;

/// 이 GUI에 번들된 제품/펌웨어 버전 정보 (프론트엔드 전달용)
#[derive(Serialize, Clone)]
pub struct ProductInfo {
    product: String,
    model: String,
    fw_version: String,
    idf_version: String,
    chip: String,
}

/// 감지된 디바이스 정보 (프론트엔드 전달용)
#[derive(Serialize, Clone)]
pub struct DeviceInfo {
    port: String,
    chip: String,
    mac: String,
    features: Vec<String>,
}

/// WiFi 주입 결과
#[derive(Serialize, Clone)]
pub struct WriteResult {
    ssid: String,
    /// NVS 저장 성공 여부 (플래시 성공 시 항상 true)
    saved: bool,
    /// 부팅 로그에서 연결 성공(IP 획득)을 확인했는지
    connected: bool,
    /// 연결 상태 코드:
    ///   "connected"      - IP 획득, 연결 완료
    ///   "ssid_not_found" - 주변에 해당 SSID 없음 (5GHz/오타 의심)
    ///   "auth_failed"    - 인증 실패 (비밀번호 오류 의심)
    ///   "trying"         - 아직 연결 중 (재시도 진행 중, 저장은 됨)
    status: String,
    /// 부팅 로그에서 추출한 IP (없으면 빈 문자열)
    ip: String,
    /// 사용자에게 보여줄 부팅 로그 요약(연결 관련 줄)
    log_excerpt: String,
}

/// 진행률 콜백 (현재는 무시 — 추후 이벤트로 프론트에 전달 가능)
struct SilentProgress;
impl ProgressCallbacks for SilentProgress {
    fn init(&mut self, _addr: u32, _total: usize) {}
    fn update(&mut self, _current: usize) {}
    fn verifying(&mut self) {}
    fn finish(&mut self, _skipped: bool) {}
}

/// 시리얼 포트를 열어 espflash 로 연결하고 Flasher 반환
fn connect_flasher(port_name: &str) -> Result<Flasher, String> {
    // 시리얼 포트 열기 (espflash 의 Port = serialport 네이티브 포트)
    let serial = serialport::new(port_name, BAUD_RATE)
        .timeout(Duration::from_secs(3))
        .open_native()
        .map_err(|e| format!("포트 열기 실패 ({}): {}", port_name, e))?;

    // USB 포트 정보 (ESP32-S3 USB JTAG)
    let port_info = serialport::UsbPortInfo {
        vid: 0,
        pid: 0,
        serial_number: None,
        manufacturer: None,
        product: None,
    };

    let connection = Connection::new(
        serial,
        port_info,
        ResetAfterOperation::HardReset,   // 플래시 후 하드리셋 → 앱 재시작
        ResetBeforeOperation::DefaultReset, // 플래시 전 부트로더 진입
        BAUD_RATE,
    );

    // use_stub=true(빠름), verify=false, skip=false, chip 자동감지, baud 유지
    Flasher::connect(connection, true, false, false, None, None)
        .map_err(|e| format!("디바이스 연결 실패: {}", e))
}

// ===================== Tauri 커맨드 =====================

/// 이 GUI에 번들된 제품/펌웨어 버전 정보 반환
#[tauri::command]
fn product_info() -> ProductInfo {
    ProductInfo {
        product: PRODUCT_NAME.to_string(),
        model: PRODUCT_MODEL.to_string(),
        fw_version: FIRMWARE_VERSION.to_string(),
        idf_version: ESP_IDF_VERSION.to_string(),
        chip: TARGET_CHIP.to_string(),
    }
}

/// 연결 후보 시리얼 포트 목록 (USB 계열만)
#[tauri::command]
fn list_ports() -> Vec<String> {
    match serialport::available_ports() {
        Ok(ports) => ports
            .into_iter()
            .map(|p| p.port_name)
            .filter(|n| {
                n.contains("usbmodem")
                    || n.contains("usbserial")
                    || n.contains("cu.")
                    || n.starts_with("/dev/ttyACM")
                    || n.starts_with("/dev/ttyUSB")
                    || n.starts_with("COM")
            })
            // tty.* 는 macOS 콜아웃용 → cu.* 선호. 단순화 위해 tty.* 제외
            .filter(|n| !n.contains("/tty."))
            .collect(),
        Err(_) => vec![],
    }
}

/// 디바이스 감지 — espflash 로 칩 정보/MAC 읽기
#[tauri::command]
fn detect_device(port: String) -> Result<DeviceInfo, String> {
    let mut flasher = connect_flasher(&port)?;
    let info = flasher
        .device_info()
        .map_err(|e| format!("디바이스 정보 읽기 실패: {}", e))?;

    // 연결 종료 (포트 해제) — 하드리셋으로 앱 재시작
    let _ = flasher.into_connection().into_serial();

    Ok(DeviceInfo {
        port,
        chip: info.chip.to_string(),
        mac: info.mac_address.unwrap_or_else(|| "?".into()),
        features: info.features,
    })
}

/// 부팅 로그 분석 결과
struct BootAnalysis {
    connected: bool,
    ip: String,
    status: String, // connected | ssid_not_found | auth_failed | trying
    excerpt: String,
}

/// 부팅 로그에서 IP / 연결 상태 / 실패 원인 추출
///
/// ESP-IDF WiFi disconnect reason 코드:
///   201 / 205 = NO_AP_FOUND  → SSID를 못 찾음 (5GHz이거나 이름 오타 의심)
///   15        = 4WAY_HANDSHAKE_TIMEOUT → 비밀번호 오류 의심
///   2 / 3     = AUTH_EXPIRE / AUTH_LEAVE → 인증 실패 의심
fn parse_boot_log(log: &str) -> BootAnalysis {
    let mut ip = String::new();
    let mut connected = false;
    let mut no_ap_found = false;
    let mut auth_failed = false;
    let mut excerpt_lines: Vec<&str> = Vec::new();

    for line in log.lines() {
        // IP 추출
        if let Some(pos) = line.find("192.168.") {
            let tail = &line[pos..];
            let ipstr: String = tail
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if ipstr.matches('.').count() == 3 {
                ip = ipstr;
            }
        }
        if line.contains("Got IP address") || line.contains("WiFi Connected Successfully") {
            connected = true;
        }
        // 끊김 사유 분석 (reason 코드)
        if line.contains("Disconnected") || line.contains("reason:") {
            if line.contains("reason: 201")
                || line.contains("reason: 205")
                || line.contains("NO_AP_FOUND")
            {
                no_ap_found = true;
            }
            if line.contains("reason: 15")
                || line.contains("reason: 2")
                || line.contains("reason: 3")
                || line.contains("HANDSHAKE")
                || line.contains("AUTH")
            {
                auth_failed = true;
            }
        }

        if line.contains("WIFI_MGR")
            || line.contains("CONFIG_MGR")
            || line.contains("192.168.")
            || line.contains("Disconnected")
            || line.contains("Connecting")
            || line.contains("reason")
        {
            excerpt_lines.push(line.trim());
        }
    }

    let excerpt = excerpt_lines
        .into_iter()
        .filter(|l| !l.is_empty())
        .take(15)
        .collect::<Vec<_>>()
        .join("\n");

    // 상태 결정 (우선순위: 연결됨 > SSID없음 > 인증실패 > 시도중)
    let status = if connected {
        "connected"
    } else if no_ap_found {
        "ssid_not_found"
    } else if auth_failed {
        "auth_failed"
    } else {
        "trying"
    }
    .to_string();

    BootAnalysis {
        connected,
        ip,
        status,
        excerpt,
    }
}

/// WiFi 설정 — NVS 바이너리 생성 후 Flash 주입, 재부팅 로그로 결과 확인
#[tauri::command]
fn write_wifi(port: String, ssid: String, password: String) -> Result<WriteResult, String> {
    // 1) NVS 바이너리 생성 (ESP-IDF 도구 불필요, 순수 Rust)
    let nvs_bin = nvs::generate_wifi_nvs(NVS_NAMESPACE, &ssid, &password, NVS_SIZE)?;

    // 2) 디바이스 연결 후 NVS 파티션에 주입
    let mut flasher = connect_flasher(&port)?;
    flasher
        .write_bin_to_flash(NVS_OFFSET, &nvs_bin, &mut SilentProgress)
        .map_err(|e| format!("NVS 주입 실패: {}", e))?;

    // 3) 플래시 종료 → 하드리셋되며 앱 재시작. 시리얼 핸들 회수.
    // 이 시점에서 NVS 저장은 이미 완료됨 (saved=true).
    let mut serial = flasher.into_connection().into_serial();

    // 4) 부팅 로그 캡처.
    //    기존 펌웨어는 바로 안 붙고 1~5초 간격으로 재시도하므로 넉넉히(최대 60초) 본다.
    //    단, 연결 성공(IP) 또는 SSID 없음(여러 번 확인)이 명확하면 조기 종료.
    let mut log = String::new();
    let mut buf = [0u8; 512];
    let start = Instant::now();
    let _ = serial.set_timeout(Duration::from_millis(300));

    while start.elapsed() < Duration::from_secs(60) {
        match serial.read(&mut buf) {
            Ok(n) if n > 0 => {
                let chunk = String::from_utf8_lossy(&buf[..n]);
                log.push_str(&chunk);

                // 연결 성공 → 조기 종료
                if log.contains("Got IP address") || log.contains("WiFi Connected Successfully") {
                    std::thread::sleep(Duration::from_millis(400));
                    if let Ok(n2) = serial.read(&mut buf) {
                        if n2 > 0 {
                            log.push_str(&String::from_utf8_lossy(&buf[..n2]));
                        }
                    }
                    break;
                }

                // SSID 못 찾음이 3회 이상 반복되면 5GHz/오타로 확정 → 조기 종료
                let no_ap_count = log.matches("reason: 201").count()
                    + log.matches("reason: 205").count()
                    + log.matches("NO_AP_FOUND").count();
                if no_ap_count >= 3 {
                    break;
                }
            }
            _ => {}
        }
    }
    drop(serial);

    let analysis = parse_boot_log(&log);

    Ok(WriteResult {
        ssid,
        saved: true, // 플래시 성공 = NVS 저장 완료
        connected: analysis.connected,
        status: analysis.status,
        ip: analysis.ip,
        log_excerpt: analysis.excerpt,
    })
}

/// 펌웨어 설치 — 번들된 부트로더/파티션테이블/앱을 각 오프셋에 플래시
///
/// ESP-IDF 없이 GUI만으로 디바이스에 펌웨어를 굽습니다.
/// (최초 셋업 또는 펌웨어 업데이트용)
#[tauri::command]
fn flash_firmware(port: String) -> Result<String, String> {
    let mut flasher = connect_flasher(&port)?;

    // 3개 바이너리를 각 오프셋에 순서대로 굽기
    let images: [(&str, u32, &[u8]); 3] = [
        ("부트로더", FW_BOOTLOADER_OFFSET, FW_BOOTLOADER),
        ("파티션 테이블", FW_PARTITION_OFFSET, FW_PARTITION),
        ("앱", FW_APP_OFFSET, FW_APP),
    ];

    for (name, offset, data) in images {
        flasher
            .write_bin_to_flash(offset, data, &mut SilentProgress)
            .map_err(|e| format!("{} 플래시 실패 (0x{:x}): {}", name, offset, e))?;
    }

    // 하드리셋되며 새 펌웨어로 부팅
    let _ = flasher.into_connection().into_serial();

    Ok(format!(
        "{} 펌웨어 v{} 설치 완료 (앱 {}KB)",
        PRODUCT_NAME,
        FIRMWARE_VERSION,
        FW_APP.len() / 1024
    ))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            product_info,
            list_ports,
            detect_device,
            flash_firmware,
            write_wifi
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 앱 실행 중 오류");
}
