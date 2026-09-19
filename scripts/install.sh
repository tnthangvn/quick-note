#!/usr/bin/env bash
# Build and install Quick Note for the current user (Ubuntu 24.04 / 26.04).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
BIN_DIR="${HOME}/.local/bin"
APP_DIR="${DATA_HOME}/applications"
ICON_DIR="${DATA_HOME}/icons/hicolor/scalable/apps"
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/quick-note"

usage() {
    cat <<EOF
Cách dùng: $(basename "$0") [TUỲ CHỌN]

  (không tuỳ chọn)   Build release và cài vào ~/.local
  -u, --uninstall    Gỡ app (giữ lại ghi chú trong ${CONFIG_DIR})
      --purge        Dùng kèm --uninstall: xoá luôn ghi chú + cài đặt (có hỏi xác nhận)
  -h, --help         Hiện trợ giúp này
EOF
}

# Absolute Exec/Icon paths: GNOME Shell hides entries whose command is not on the PATH it
# got at login, and ~/.local/bin is often added to PATH only after a re-login.
install_desktop_entry() {
    mkdir -p "${APP_DIR}"
    sed -e "s|^Exec=.*|Exec=${BIN_DIR}/quick-note|" \
        -e "s|^Icon=.*|Icon=${ICON_DIR}/quick-note.svg|" \
        "${ROOT}/assets/quick-note.desktop" >"${APP_DIR}/quick-note.desktop"
    chmod 644 "${APP_DIR}/quick-note.desktop"
    if command -v desktop-file-validate >/dev/null 2>&1; then
        desktop-file-validate "${APP_DIR}/quick-note.desktop"
    fi
}

refresh_caches() {
    update-desktop-database "${APP_DIR}" >/dev/null 2>&1 || true
    gtk-update-icon-cache -q "${DATA_HOME}/icons/hicolor" >/dev/null 2>&1 || true
}

uninstall() {
    local purge="$1"
    pkill -x quick-note 2>/dev/null || true
    for f in "${BIN_DIR}/quick-note" "${APP_DIR}/quick-note.desktop" "${ICON_DIR}/quick-note.svg"; do
        if [[ -e "$f" ]]; then
            rm -f -- "$f"
            echo "Đã xoá $f"
        fi
    done
    refresh_caches

    if [[ ! -d "${CONFIG_DIR}" ]]; then
        return
    fi
    if [[ "$purge" != 1 ]]; then
        echo "Giữ lại ghi chú ở ${CONFIG_DIR} (thêm --purge để xoá)."
        return
    fi
    read -r -p "Xoá VĨNH VIỄN toàn bộ ghi chú, cài đặt và token Drive trong ${CONFIG_DIR}? Gõ \"yes\" để xác nhận: " answer
    if [[ "$answer" == "yes" ]]; then
        rm -rf -- "${CONFIG_DIR}"
        echo "Đã xoá ${CONFIG_DIR}. Bản trên Google Drive (nếu có) không bị xoá."
    else
        echo "Đã huỷ, dữ liệu được giữ nguyên."
    fi
}

install_app() {
    if ! command -v cargo >/dev/null 2>&1; then
        echo "cargo không có. Cài Rust: curl https://sh.rustup.rs -sSf | sh" >&2
        exit 1
    fi
    if ! command -v cc >/dev/null 2>&1; then
        echo "Thiếu C compiler (cần cho TLS). Chạy: sudo apt install build-essential cmake" >&2
        exit 1
    fi

    cargo build --release --manifest-path "${ROOT}/Cargo.toml"

    # Stop a running copy so the binary can be replaced and the new version starts clean.
    pkill -x quick-note 2>/dev/null || true
    install -Dm755 "${ROOT}/target/release/quick-note" "${BIN_DIR}/quick-note"
    install -Dm644 "${ROOT}/assets/quick-note.svg" "${ICON_DIR}/quick-note.svg"
    install_desktop_entry
    refresh_caches

    echo "Đã cài: ${BIN_DIR}/quick-note"
    echo "Mở từ menu ứng dụng: bấm Super, gõ \"Quick Note\" (nếu chưa thấy, đợi vài giây hoặc đăng xuất/đăng nhập lại)."
    case ":${PATH}:" in
        *":${BIN_DIR}:"*) ;;
        *) echo "Lưu ý: thêm ${BIN_DIR} vào PATH (Ubuntu thường tự thêm sau khi đăng nhập lại)." ;;
    esac
}

main() {
    local mode=install purge=0
    for arg in "$@"; do
        case "$arg" in
            -h | --help) usage; exit 0 ;;
            -u | --uninstall) mode=uninstall ;;
            --purge) purge=1 ;;
            *) echo "Tuỳ chọn không hợp lệ: $arg" >&2; usage >&2; exit 1 ;;
        esac
    done
    if [[ "$purge" == 1 && "$mode" != uninstall ]]; then
        echo "--purge chỉ dùng kèm --uninstall" >&2
        exit 1
    fi
    if [[ "$mode" == uninstall ]]; then
        uninstall "$purge"
    else
        install_app
    fi
}

# `exit` on the same line: bash never reads past here, so editing this file while
# it runs (e.g. during the long cargo build) cannot break the running copy.
main "$@"; exit
