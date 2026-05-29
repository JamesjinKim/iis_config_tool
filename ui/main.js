// IIS3DWB WiFi 설정 툴 — 프론트엔드 로직 (NVS 직접 주입 방식)
// Tauri 백엔드(Rust)의 커맨드를 invoke 로 호출합니다.
//   list_ports()                       → 후보 포트 목록
//   detect_device(port)                → 칩/MAC 정보
//   write_wifi(port, ssid, password)   → NVS 주입 + 부팅 로그로 결과 확인

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

let selectedPort = null; // 사용자가 선택/감지한 포트

const $ = (id) => document.getElementById(id);
const overlay = $("overlay");
const overlayText = $("overlay-text");

function showOverlay(text) {
  overlayText.textContent = text;
  overlay.classList.remove("hidden");
}
function hideOverlay() {
  overlay.classList.add("hidden");
}

// 화면 전환 + 스테퍼 갱신
function goStep(n) {
  document.querySelectorAll(".screen").forEach((s) => s.classList.remove("active"));
  $(`screen-${n}`).classList.add("active");
  document.querySelectorAll(".step").forEach((el) => {
    const step = Number(el.dataset.step);
    el.classList.remove("active", "done");
    if (step < n) el.classList.add("done");
    else if (step === n) el.classList.add("active");
  });
}

// ===== 포트 목록 갱신 =====
async function refreshPorts() {
  const sel = $("port-select");
  try {
    const ports = await invoke("list_ports");
    sel.innerHTML = "";
    if (!ports.length) {
      sel.innerHTML = `<option value="">포트를 찾을 수 없음</option>`;
      return;
    }
    for (const p of ports) {
      const opt = document.createElement("option");
      opt.value = p;
      opt.textContent = p;
      sel.appendChild(opt);
    }
  } catch (e) {
    sel.innerHTML = `<option value="">오류: ${e}</option>`;
  }
}

$("btn-refresh-ports").addEventListener("click", refreshPorts);

// ===== 1단계: 디바이스 연결 =====
$("btn-detect").addEventListener("click", async () => {
  const port = $("port-select").value;
  const statusBox = $("device-status");
  if (!port) {
    statusBox.className = "device-box error";
    statusBox.innerHTML = `<div class="title">⚠ 포트를 선택하세요</div>`;
    statusBox.classList.remove("hidden");
    return;
  }

  showOverlay("디바이스에 연결하는 중...");
  try {
    const info = await invoke("detect_device", { port });
    hideOverlay();
    selectedPort = info.port;

    const features = (info.features || []).join(", ");
    statusBox.className = "device-box";
    statusBox.innerHTML = `
      <div class="title">✅ 디바이스 감지됨</div>
      <div class="row">칩: <b>${info.chip}</b></div>
      <div class="row">MAC: <code>${info.mac}</code></div>
      <div class="row">기능: ${features}</div>
      <div class="row">포트: <code>${info.port}</code></div>`;
    statusBox.classList.remove("hidden");

    $("btn-detect").textContent = "다시 연결";
    $("btn-to-wifi").classList.remove("hidden");
    $("btn-flash-fw").classList.remove("hidden"); // 펌웨어 설치 옵션 노출
  } catch (e) {
    hideOverlay();
    statusBox.className = "device-box error";
    statusBox.innerHTML = `
      <div class="title">❌ 연결 실패</div>
      <div class="row">${e}</div>
      <div class="row">USB 연결을 확인하고, 다른 프로그램(시리얼 모니터 등)이
        포트를 쓰고 있지 않은지 확인하세요.</div>`;
    statusBox.classList.remove("hidden");
    $("btn-to-wifi").classList.add("hidden");
    $("btn-flash-fw").classList.add("hidden");
  }
});

$("btn-to-wifi").addEventListener("click", () => goStep(2));

// ===== 펌웨어 설치 / 업데이트 =====
$("btn-flash-fw").addEventListener("click", async () => {
  if (!selectedPort) return;
  // 사용자 확인 (펌웨어 덮어쓰기이므로)
  const ok = confirm(
    "디바이스에 펌웨어를 설치(업데이트)합니다.\n" +
    "1~2분 정도 걸리며, 완료 후 자동 재부팅됩니다.\n\n" +
    "진행할까요?"
  );
  if (!ok) return;

  showOverlay("펌웨어 설치 중... (1~2분 소요, USB를 분리하지 마세요)");
  try {
    const msg = await invoke("flash_firmware", { port: selectedPort });
    hideOverlay();
    const statusBox = $("device-status");
    statusBox.className = "device-box";
    statusBox.innerHTML = `
      <div class="title">✅ 펌웨어 설치 완료</div>
      <div class="row">${msg}</div>
      <div class="row">디바이스가 재부팅되었습니다. 이제 <b>WiFi 설정 ▶</b>으로 진행하세요.</div>`;
    statusBox.classList.remove("hidden");
  } catch (e) {
    hideOverlay();
    const statusBox = $("device-status");
    statusBox.className = "device-box error";
    statusBox.innerHTML = `
      <div class="title">❌ 펌웨어 설치 실패</div>
      <div class="row">${e}</div>
      <div class="row">USB 연결을 확인하고 다시 시도하세요.</div>`;
    statusBox.classList.remove("hidden");
  }
});

// 비밀번호 표시 토글
$("btn-show-pw").addEventListener("click", () => {
  const pw = $("wifi-pw");
  pw.type = pw.type === "password" ? "text" : "password";
});

function showWifiError(msg) {
  const box = $("wifi-error");
  box.textContent = msg;
  box.classList.remove("hidden");
}
function clearWifiError() {
  $("wifi-error").classList.add("hidden");
}

// ===== 2단계: 저장 & 연결 확인 =====
$("btn-save").addEventListener("click", async () => {
  clearWifiError();
  const ssid = $("wifi-ssid").value.trim();
  const pw = $("wifi-pw").value;

  if (!ssid) {
    showWifiError("WiFi 이름(SSID)을 입력하세요.");
    return;
  }
  if (!selectedPort) {
    showWifiError("디바이스 포트가 없습니다. 1단계로 돌아가세요.");
    return;
  }

  showOverlay("WiFi 정보를 저장하고 연결을 확인하는 중...\n(연결은 보통 수~수십 초 걸립니다. 최대 60초 대기)");
  try {
    const result = await invoke("write_wifi", {
      port: selectedPort,
      ssid,
      password: pw,
    });
    hideOverlay();

    const icon = $("result-icon");
    const title = $("result-title");
    const sub = $("result-sub");
    const box = $("result-box");

    // status: connected | ssid_not_found | auth_failed | trying
    switch (result.status) {
      case "connected":
        icon.textContent = "✅";
        title.textContent = "설정 완료 & WiFi 연결됨!";
        sub.textContent = "이제 USB를 분리해도 됩니다. 전원만 들어오면 자동으로 이 WiFi에 연결됩니다.";
        box.className = "result-box";
        box.innerHTML = `
          <div class="row"><b>WiFi:</b> <span class="val">${result.ssid}</span></div>
          <div class="row"><b>IP 주소:</b> <span class="val">${result.ip || "(획득 중)"}</span></div>`;
        break;

      case "ssid_not_found":
        icon.textContent = "❌";
        title.textContent = "WiFi를 찾을 수 없습니다";
        sub.textContent = "주변에서 해당 WiFi가 검색되지 않았습니다. 저장은 되었지만 연결되지 않습니다.";
        box.className = "result-box warn";
        box.innerHTML = `
          <div class="row"><b>입력한 WiFi:</b> <span class="val">${result.ssid}</span></div>
          <div class="row">⚠️ 다음을 확인하세요:</div>
          <div class="row">• <b>2.4GHz</b> WiFi인가요? (5GHz는 지원 안 함 — 예: 이름 끝 '5G')</div>
          <div class="row">• WiFi 이름의 <b>대소문자/공백</b>이 정확한가요?</div>
          <div class="row">• 디바이스가 신호 <b>범위 안</b>에 있나요?</div>
          <div class="row">→ <b>다시 설정</b>으로 올바른 WiFi를 입력하세요.</div>`;
        break;

      case "auth_failed":
        icon.textContent = "🔑";
        title.textContent = "비밀번호 오류로 보입니다";
        sub.textContent = "WiFi는 찾았지만 인증에 실패했습니다. 저장은 되었지만 연결되지 않습니다.";
        box.className = "result-box warn";
        box.innerHTML = `
          <div class="row"><b>WiFi:</b> <span class="val">${result.ssid}</span></div>
          <div class="row">🔑 <b>비밀번호</b>를 다시 확인하세요. (👁 아이콘으로 확인 가능)</div>
          <div class="row">→ <b>다시 설정</b>으로 올바른 비밀번호를 입력하세요.</div>`;
        break;

      default: // "trying"
        icon.textContent = "⏳";
        title.textContent = "저장 완료 — 연결 시도 중";
        sub.textContent = "WiFi 정보는 저장됐습니다. 디바이스가 계속 연결을 재시도하고 있습니다.";
        box.className = "result-box warn";
        box.innerHTML = `
          <div class="row"><b>저장된 WiFi:</b> <span class="val">${result.ssid}</span></div>
          <div class="row">⏳ 신호가 약하거나 공유기가 바쁘면 연결까지 시간이 더 걸립니다.</div>
          <div class="row">디바이스는 자동으로 계속 시도합니다. 잠시 후 공유기 관리페이지에서
            연결 여부를 확인하거나, 신호가 강한 곳으로 옮겨보세요.</div>`;
        break;
    }

    // 부팅 로그
    $("log-pre").textContent = result.log_excerpt || "(로그 없음)";

    // 결과에 따라 버튼 구성: 성공이면 [종료] 강조, 실패면 [다시 설정] 강조
    configureFinishButtons(result.status === "connected");

    goStep(3);
  } catch (e) {
    hideOverlay();
    showWifiError(String(e));
  }
});

$("btn-back-1").addEventListener("click", () => goStep(1));

// ===== 3단계: 완료 — 결과에 따라 버튼 강조/순서 결정 =====
//
// 성공: [종료](강조) + [다른 디바이스 설정](보조)   — 할 일 끝남
// 실패: [다시 설정](강조) + [종료](보조)            — 값을 고쳐야 함
function configureFinishButtons(isSuccess) {
  const retry = $("btn-retry");   // 다시 설정 (2단계로)
  const quit = $("btn-quit");     // 앱 종료
  const another = $("btn-another"); // 다른 디바이스 설정 (1단계로)

  if (isSuccess) {
    // 종료를 주(primary) 버튼으로, 다시 설정은 숨김
    quit.classList.remove("hidden", "ghost");
    quit.classList.add("primary");
    retry.classList.add("hidden");
    another.classList.remove("hidden");
  } else {
    // 다시 설정을 주(primary) 버튼으로, 종료는 보조(ghost)
    retry.classList.remove("hidden", "ghost");
    retry.classList.add("primary");
    quit.classList.remove("primary");
    quit.classList.add("ghost");
    quit.classList.remove("hidden");
    another.classList.add("hidden");
  }
}

// 다시 설정 → 2단계 (비밀번호만 비우고 SSID는 유지해 재입력 편하게)
$("btn-retry").addEventListener("click", () => {
  $("wifi-pw").value = "";
  goStep(2);
});

// 다른 디바이스 설정 → 1단계 (입력값 초기화)
$("btn-another").addEventListener("click", () => {
  $("wifi-pw").value = "";
  $("wifi-ssid").value = "";
  goStep(1);
});

// 종료 → 앱 완전 종료
$("btn-quit").addEventListener("click", async () => {
  try {
    await getCurrentWindow().close();
  } catch (e) {
    // 닫기 실패 시 최소한 1단계로
    goStep(1);
  }
});

// ===== 시작 시: 제품/버전 정보 로드 → 푸터 표시 =====
async function loadProductInfo() {
  try {
    const info = await invoke("product_info");
    $("version-bar").textContent =
      `${info.product} · 펌웨어 v${info.fw_version} · ${info.chip} · ESP-IDF ${info.idf_version}`;
    // 1단계 안내 문구에도 제품명 반영
    document.title = `${info.product} 설정`;
  } catch (e) {
    $("version-bar").textContent = "버전 정보를 불러오지 못했습니다";
  }
}

// 앱 시작 시 버전 정보 + 포트 목록 자동 로드
loadProductInfo();
refreshPorts();
