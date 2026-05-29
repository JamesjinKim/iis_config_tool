# IIS3DWB WiFi 설정 시스템 (config-tool)

IIS3DWB 진동센서 디바이스의 WiFi를 설정하고 디바이스 Flash(NVS)에 영구 저장하는
**설정 도구와 문서**를 모아둔 폴더입니다.

> ✅ **검증 완료:** PC에서 WiFi 값이 담긴 NVS 바이너리를 만들어 Flash에 직접 주입하면,
> 디바이스가 부팅 시 그 값을 읽어 **WiFi에 자동 연결**합니다. 실기기에서 IP 획득까지 확인했습니다.

---

## 📁 폴더 구조

```
config-tool/
├── README.md          ← 이 파일
├── docs/
│   └── index.html     ← 전체 시스템 문서 (브라우저로 열기)
├── nvs_wifi.csv       ← WiFi 값 입력 (key,type,encoding,value)
├── nvs_wifi.bin       ← CSV에서 생성된 NVS 바이너리 (Flash 주입용)
├── ui/ , src-tauri/   ← macOS GUI 설정 툴 (Tauri) — 빌드 완료
└── package.json , vite.config.js
```

---

## 📖 문서 보기

| 문서 | 용도 |
|------|------|
| `docs/setup-guide.html` | 🛠️ **셋업 설명서** — 펌웨어 빌드/플래시 (최초 1회) |
| `docs/user-guide.html` | 🖥️ **GUI 사용 설명서** — WiFi 설정, **선택/입력값 안내** |
| `docs/index.html` | 📘 전체 시스템 문서 — 구조·NVS·검증 로그 |

```bash
open docs/user-guide.html    # GUI 따라하기 (선택값 안내 포함)
open docs/setup-guide.html   # 펌웨어 플래시
open docs/index.html         # 전체 시스템 문서
```

---

## ⚡ 빠른 사용법 — NVS 직접 주입 (검증된 방법)

WiFi 값을 NVS 형식으로 만들어 Flash에 굽습니다. 실시간 통신 없이 자동 연결됩니다.

```bash
get_idf
cd /Users/kimkookjin/Projects/ESP-IDF/IIS3DWB/config-tool

# 1) nvs_wifi.csv 의 SSID/비밀번호를 원하는 값으로 수정

# 2) NVS 바이너리 생성 (24K = 0x6000)
python $IDF_PATH/components/nvs_flash/nvs_partition_generator/nvs_partition_gen.py \
       generate nvs_wifi.csv nvs_wifi.bin 0x6000

# 3) NVS 파티션(0x9000)에 주입 — 펌웨어는 유지됨
esptool.py --chip esp32s3 -p /dev/cu.usbmodem1433301 -b 460800 \
           write_flash 0x9000 nvs_wifi.bin

# 4) 결과 확인 (★ WiFi Connected Successfully! + IP 표시)
idf.py -p /dev/cu.usbmodem1433301 monitor
```

### NVS CSV 형식
```
key,type,encoding,value
devcfg,namespace,,
wifi_ssid,data,string,example2.4G
wifi_pass,data,string,example1234
```

> 네임스페이스 `devcfg`, 키 `wifi_ssid`/`wifi_pass`는 펌웨어 `config_manager.c`와
> **정확히 일치**해야 합니다. 문자열은 `type=data, encoding=string`.

### NVS 파티션 정보
| 항목 | 값 |
|------|-----|
| 파티션 | `nvs` |
| 오프셋 | `0x9000` |
| 크기 | `24K` (0x6000) |

---

## 🖥️ GUI 설정 툴 (Tauri)

macOS GUI 툴도 구현·빌드되어 있습니다 (3단계 마법사 UI).

```bash
npm install
npm run tauri dev      # 개발 모드 실행
npm run tauri build    # 배포용 .app/.dmg 생성
```

> 현재 GUI 툴은 USB 시리얼(RX) 통신 기반입니다. 향후 위의 **NVS 직접 주입 방식**을
> 툴 안에 통합하면, 실시간 통신 없이도 사용자 설정이 가능합니다 (다음 단계 참고).

---

## ⚙️ 펌웨어 측 코드 위치 (참고)

펌웨어 컴포넌트는 ESP-IDF 빌드에 묶여 프로젝트 루트의 `components/`에 있습니다.

| 컴포넌트 | 역할 |
|----------|------|
| `../components/config_manager/` | NVS(Flash) WiFi 저장/로드 (네임스페이스 `devcfg`) |
| `../components/wifi_manager/` | WiFi 연결 (미설정 시 자동연결 안 함) |
| `../components/serial_protocol/` | USB JSON 명령 처리 (RX 통신용) |

---

## 🚧 다음 단계

- [ ] PC 툴에 **NVS 직접 주입** 통합 (사용자 입력 → bin 생성 → esptool 굽기 자동화)
- [ ] NVS 암호화 (비밀번호 평문 저장 방지, 양산 전)
- [ ] (선택) USB RX 실시간 통신 또는 BLE — 케이블 굽기 없는 즉시 변경
- [ ] Windows / Linux 확장
```
