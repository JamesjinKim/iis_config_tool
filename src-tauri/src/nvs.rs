//! ESP-IDF NVS 파티션 바이너리 생성기 (V2 포맷)
//!
//! WiFi SSID/비밀번호를 NVS 형식의 바이너리로 변환합니다.
//! 이 바이너리를 Flash의 NVS 파티션(0x9000)에 그대로 쓰면,
//! 펌웨어가 `nvs_get_str`로 읽을 수 있습니다.
//!
//! 참고: ESP-IDF nvs_partition_gen.py 가 생성한 바이너리와 동일한 구조를
//! 재현합니다. (검증된 nvs_wifi.bin 의 바이트 레이아웃 기준)
//!
//! 페이지 구조 (4096 바이트):
//!   - 헤더 32B: state(4) + seqno(4) + version(1) + ... + crc32(4)
//!   - 엔트리 상태 비트맵 32B: 엔트리당 2비트 (10=written, 11=empty)
//!   - 엔트리 126개 × 32B
//!
//! 엔트리(32B): ns(1) type(1) span(1) chunk_idx(1) crc32(4) key(16) data(8)

// NVS 의 모든 CRC32 는 nvs_partition_gen.py 의 zlib.crc32(data, 0xFFFFFFFF) 와 동일하다.
// crc 크레이트의 ISO-HDLC 는 init 인자를 줄 수 없어, zlib 호환 순수 구현을 사용한다.
// (실측: 아래 구현이 기준 bin 의 헤더/엔트리/데이터 CRC 와 모두 바이트 일치)

/// zlib.crc32(data, init) 호환 구현 (reflected CRC-32, poly 0xEDB88320).
/// init 을 이어받아 여러 조각을 연속 계산할 수 있다.
fn zlib_crc32(data: &[u8], init: u32) -> u32 {
    let mut crc = init ^ 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc ^ 0xFFFF_FFFF
}

/// NVS 표준 시작값(0xFFFFFFFF)으로 단일 버퍼 CRC32
fn nvs_crc(data: &[u8]) -> u32 {
    zlib_crc32(data, 0xFFFF_FFFF)
}

const PAGE_SIZE: usize = 4096;
const ENTRY_SIZE: usize = 32;
const ENTRIES_PER_PAGE: usize = 126; // (4096 - 32 - 32) / 32
const NS_INDEX: u8 = 1; // devcfg 네임스페이스의 인덱스

// NVS 엔트리 타입
const TYPE_U8: u8 = 0x01;
const TYPE_STR: u8 = 0x21;

/// 하나의 NVS 페이지를 만드는 빌더
struct NvsPage {
    entries: Vec<[u8; ENTRY_SIZE]>, // 채워진 엔트리들
    states: Vec<bool>,              // 각 엔트리 written 여부 (true=written)
}

impl NvsPage {
    fn new() -> Self {
        NvsPage {
            entries: Vec::new(),
            states: Vec::new(),
        }
    }

    /// 엔트리 1개의 CRC32 계산.
    /// ESP-IDF: 헤더의 ns/type/span/chunk_idx + key(16) + data(8) 부분을 대상으로 하되,
    /// crc 필드(4바이트)는 제외하고 계산한다.
    fn entry_crc(entry: &[u8; ENTRY_SIZE]) -> u32 {
        // crc 대상: [0..4] (ns,type,span,chunk) + [8..32] (key+data)
        // crc 필드는 [4..8]
        // crc 대상 두 조각을 연속 계산: [0..4] 다음 [8..32]
        let c = zlib_crc32(&entry[0..4], 0xFFFF_FFFF);
        zlib_crc32(&entry[8..32], c)
    }

    /// namespace 엔트리 추가 (type U8, data[0]=ns_index)
    fn add_namespace(&mut self, name: &str, ns_index: u8) {
        let mut e = [0xFFu8; ENTRY_SIZE];
        e[0] = 0; // ns_index 0 = namespace 엔트리 자체
        e[1] = TYPE_U8;
        e[2] = 1; // span
        e[3] = 0xFF; // chunk_idx
        // key (16바이트, NULL 패딩)
        write_key(&mut e, name);
        // data (8바이트): 첫 바이트가 ns_index 값, 나머지는 0xFF (기준 bin과 일치)
        e[24] = ns_index;
        // e[25..32] 는 초기값 0xFF 유지
        let crc = Self::entry_crc(&e);
        e[4..8].copy_from_slice(&crc.to_le_bytes());
        self.entries.push(e);
        self.states.push(true);
    }

    /// string 엔트리 추가 (헤더 엔트리 + 데이터 엔트리들)
    fn add_string(&mut self, key: &str, value: &str) {
        // ESP-IDF 문자열은 NULL 종료 포함하여 저장
        let mut data = value.as_bytes().to_vec();
        data.push(0); // NULL 종료
        let data_len = data.len() as u16;

        // 데이터가 차지하는 엔트리 수 (8바이트 단위 올림)
        let data_entries = (data.len() + ENTRY_SIZE - 1) / ENTRY_SIZE;
        let span = (1 + data_entries) as u8;

        // 데이터 CRC32 (저장될 데이터 전체)
        let data_crc = nvs_crc(&data);

        // 1) 헤더 엔트리
        let mut head = [0xFFu8; ENTRY_SIZE];
        head[0] = NS_INDEX;
        head[1] = TYPE_STR;
        head[2] = span;
        head[3] = 0xFF; // chunk_idx
        write_key(&mut head, key);
        // data 필드(8B): size(2 LE) + reserved(2)=0xFFFF + data_crc32(4 LE)
        head[24..26].copy_from_slice(&data_len.to_le_bytes());
        head[26] = 0xFF;
        head[27] = 0xFF;
        head[28..32].copy_from_slice(&data_crc.to_le_bytes());
        let crc = Self::entry_crc(&head);
        head[4..8].copy_from_slice(&crc.to_le_bytes());
        self.entries.push(head);
        self.states.push(true);

        // 2) 데이터 엔트리들 (32바이트씩, 남는 부분 0xFF 패딩)
        let mut padded = data.clone();
        while padded.len() % ENTRY_SIZE != 0 {
            padded.push(0xFF);
        }
        for chunk in padded.chunks(ENTRY_SIZE) {
            let mut e = [0xFFu8; ENTRY_SIZE];
            e.copy_from_slice(chunk);
            self.entries.push(e);
            self.states.push(true);
        }
    }

    /// 페이지를 4096바이트 바이너리로 직렬화
    fn serialize(&self) -> Vec<u8> {
        let mut page = vec![0xFFu8; PAGE_SIZE];

        // --- 페이지 헤더 (32B) ---
        // state: 0xFFFFFFFE = ACTIVE
        page[0..4].copy_from_slice(&[0xFE, 0xFF, 0xFF, 0xFF]);
        // seqno: 0
        page[4..8].copy_from_slice(&0u32.to_le_bytes());
        // version: 0xFE = V2 (offset 8)
        page[8] = 0xFE;
        // [9..28] 0xFF 유지
        // 헤더 CRC32: 대상 [4..28] (seqno~version~unused), crc 필드는 [28..32]
        let hdr_crc = nvs_crc(&page[4..28]);
        page[28..32].copy_from_slice(&hdr_crc.to_le_bytes());

        // --- 엔트리 상태 비트맵 (32B @ offset 32) ---
        // 엔트리당 2비트: 0b10=written, 0b11=empty(미사용)
        // 비트맵 전체를 0xFF(모두 empty)로 시작 후, written 엔트리만 10으로 설정
        for i in 0..ENTRIES_PER_PAGE {
            let written = self.states.get(i).copied().unwrap_or(false);
            if written {
                // empty(11) → written(10): 하위 비트를 0으로
                let byte_idx = 32 + (i / 4);
                let bit_pos = (i % 4) * 2;
                // 해당 2비트를 10으로: 먼저 11로 되어있으니 LSB쪽 비트를 클리어
                page[byte_idx] &= !(1 << bit_pos);
            }
        }

        // --- 엔트리들 (@ offset 64) ---
        for (i, entry) in self.entries.iter().enumerate() {
            let off = 64 + i * ENTRY_SIZE;
            page[off..off + ENTRY_SIZE].copy_from_slice(entry);
        }

        page
    }
}

/// 16바이트 key 필드에 문자열 기록 (NULL 패딩)
fn write_key(entry: &mut [u8; ENTRY_SIZE], key: &str) {
    let kb = key.as_bytes();
    let n = kb.len().min(15); // 최대 15 + NULL
    entry[8..8 + n].copy_from_slice(&kb[..n]);
    for b in entry.iter_mut().take(24).skip(8 + n) {
        *b = 0;
    }
}

/// WiFi SSID/비밀번호로 NVS 파티션 바이너리 생성
///
/// # Arguments
/// * `namespace` - NVS 네임스페이스 (펌웨어와 일치: "devcfg")
/// * `ssid` - WiFi SSID
/// * `password` - WiFi 비밀번호
/// * `partition_size` - NVS 파티션 크기 (바이트, 예: 0x6000 = 24576)
///
/// # Returns
/// NVS 바이너리 (partition_size 바이트, 미사용 영역 0xFF)
pub fn generate_wifi_nvs(
    namespace: &str,
    ssid: &str,
    password: &str,
    partition_size: usize,
) -> Result<Vec<u8>, String> {
    if partition_size < PAGE_SIZE * 2 {
        return Err("NVS 파티션 크기가 너무 작습니다 (최소 8KB)".into());
    }
    if partition_size % PAGE_SIZE != 0 {
        return Err("NVS 파티션 크기는 4096의 배수여야 합니다".into());
    }
    if ssid.is_empty() || ssid.len() > 32 {
        return Err("SSID 길이가 올바르지 않습니다 (1~32)".into());
    }
    if password.len() > 64 {
        return Err("비밀번호가 너무 깁니다 (최대 64)".into());
    }

    // 첫 페이지에 namespace + 두 개의 string 엔트리 작성
    let mut page = NvsPage::new();
    page.add_namespace(namespace, NS_INDEX);
    page.add_string("wifi_ssid", ssid);
    page.add_string("wifi_pass", password);

    let first = page.serialize();

    // 전체 파티션: 첫 페이지(데이터) + 나머지 페이지(빈 페이지 = 전부 0xFF)
    let mut out = vec![0xFFu8; partition_size];
    out[0..PAGE_SIZE].copy_from_slice(&first);

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ESP-IDF nvs_partition_gen.py 가 생성한 검증 bin과 바이트 단위 비교
    #[test]
    fn matches_reference_bin() {
        let reference = include_bytes!("../tests_ref_nvs.bin");
        let generated = generate_wifi_nvs("devcfg", "example2.4G", "example1234", 0x6000)
            .expect("생성 실패");

        assert_eq!(generated.len(), reference.len(), "크기 불일치");

        // 전체 24K 바이트 단위 비교
        for i in 0..reference.len() {
            if generated[i] != reference[i] {
                panic!(
                    "오프셋 0x{:04x} 에서 첫 차이: 생성={:02x} 기준={:02x}",
                    i, generated[i], reference[i]
                );
            }
        }
    }
}
