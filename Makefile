# Quick Note — lệnh hay dùng. Gõ `make` hoặc `make help` để xem danh sách.
#
# Quy ước: dòng `target: ## mô tả` tự hiện trong `make help`.

SHELL      := /usr/bin/env bash
.DEFAULT_GOAL := help

APP        := quick-note
INSTALL_SH := ./scripts/install.sh
BIN        := $(HOME)/.local/bin/$(APP)
CONFIG_DIR := $(or $(XDG_CONFIG_HOME),$(HOME)/.config)/$(APP)
BRANCH     := $(shell git rev-parse --abbrev-ref HEAD 2>/dev/null || echo main)

.PHONY: help deps build release run run-window test check clippy fmt lint \
        install reinstall upgrade uninstall purge deb \
        clean clean-all clean-cache clean-attachments \
        start stop restart status version

help: ## Hiện danh sách lệnh
	@echo "Quick Note — make <lệnh>"
	@echo
	@awk 'BEGIN {FS = ":.*## "} /^[a-zA-Z0-9_-]+:.*## / { printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2 }' $(MAKEFILE_LIST)
	@echo
	@echo "Ví dụ: make install        make upgrade        make clean-cache"

# ---------------------------------------------------------------- Phát triển

deps: ## Cài gói hệ thống cần để build (apt + rustup nếu thiếu)
	sudo apt install -y build-essential cmake pkg-config libxkbcommon-dev libwayland-dev libx11-dev libegl1-mesa-dev
	@command -v cargo >/dev/null || { echo "Cài Rust: curl https://sh.rustup.rs -sSf | sh"; exit 1; }

build: ## Build debug
	cargo build

release: ## Build release (target/release/quick-note)
	cargo build --release

run: ## Chạy bản debug (sticky góc màn hình)
	cargo run

run-window: ## Chạy bản debug ở cửa sổ thường (-w)
	cargo run -- --window

test: ## Chạy test
	cargo test

check: ## cargo check nhanh, không build binary
	cargo check --all-targets

clippy: ## Lint với clippy (warning = lỗi)
	cargo clippy --all-targets -- -D warnings

fmt: ## Format code
	cargo fmt

lint: fmt clippy test ## fmt + clippy + test (chạy trước khi commit)

# ---------------------------------------------------------------- Cài đặt

install: ## Build release và cài vào ~/.local (tự tắt bản đang chạy)
	$(INSTALL_SH)

reinstall: ## Xoá sạch target/ rồi build + cài lại từ đầu (lâu)
	$(INSTALL_SH) --full-clean

upgrade: ## git pull nhánh hiện tại rồi build + cài lại
	@git diff --quiet || { echo "Có thay đổi chưa commit, stash/commit trước khi upgrade."; exit 1; }
	git pull --ff-only origin $(BRANCH)
	$(INSTALL_SH)

uninstall: ## Gỡ app, giữ lại ghi chú trong ~/.config/quick-note
	$(INSTALL_SH) --uninstall

purge: ## Gỡ app VÀ xoá toàn bộ ghi chú/cài đặt/token (có hỏi xác nhận)
	$(INSTALL_SH) --uninstall --purge

deb: ## Đóng gói .deb (cần cargo-deb) -> target/debian/
	@command -v cargo-deb >/dev/null || cargo install cargo-deb
	cargo deb
	@ls -1 target/debian/*.deb

# ---------------------------------------------------------------- Dọn dẹp

clean: ## Xoá build của app, giữ cache thư viện (build lại nhanh)
	cargo clean -p $(APP)
	cargo clean -p $(APP) --release

clean-all: ## Xoá sạch target/ (kể cả thư viện, build lại mất vài phút)
	cargo clean

clean-cache: ## Dọn cache desktop/icon/menu khi icon hoặc mục menu không cập nhật
	$(INSTALL_SH) --refresh-caches

clean-attachments: ## Xoá ảnh đã dán không còn được note nào tham chiếu
	@dir="$(CONFIG_DIR)/attachments"; \
	[[ -d "$$dir" ]] || { echo "Không có $$dir"; exit 0; }; \
	n=0; \
	for f in "$$dir"/*; do \
		[[ -f "$$f" ]] || continue; \
		grep -rqF "$$(basename "$$f")" "$(CONFIG_DIR)" --include='*.json' 2>/dev/null && continue; \
		rm -f -- "$$f"; n=$$((n+1)); echo "  xoá $$(basename "$$f")"; \
	done; \
	echo "Đã xoá $$n ảnh mồ côi."

# ---------------------------------------------------------------- Chạy/dừng

start: ## Mở bản đã cài (chạy nền, tách khỏi terminal)
	@[[ -x "$(BIN)" ]] || { echo "Chưa cài, chạy: make install"; exit 1; }
	@nohup "$(BIN)" >/dev/null 2>&1 & disown; echo "Đã mở $(APP)."

stop: ## Tắt bản đang chạy
	@pkill -x $(APP) && echo "Đã tắt $(APP)." || echo "$(APP) không chạy."

restart: stop start ## Tắt rồi mở lại

status: ## Trạng thái: tiến trình, đường dẫn binary, thư mục dữ liệu
	@pgrep -x $(APP) >/dev/null && echo "Đang chạy (pid $$(pgrep -x $(APP) | tr '\n' ' '))" || echo "Không chạy"
	@[[ -x "$(BIN)" ]] && echo "Binary:  $(BIN) ($$("$(BIN)" --version 2>/dev/null))" || echo "Binary:  chưa cài"
	@echo "Dữ liệu: $(CONFIG_DIR)"
	@[[ -d "$(CONFIG_DIR)" ]] && du -sh "$(CONFIG_DIR)" 2>/dev/null | awk '{print "Dung lượng: " $$1}' || true

version: ## Phiên bản trong Cargo.toml và bản đã cài
	@echo "Cargo.toml: $$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
	@[[ -x "$(BIN)" ]] && echo "Đã cài:     $$("$(BIN)" --version 2>/dev/null)" || echo "Đã cài:     chưa cài"
