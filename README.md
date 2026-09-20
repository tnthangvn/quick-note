# Quick Note

App ghi chú nhanh dạng sticky note cho Ubuntu 24.04 / 26.04 (Wayland và X11), viết bằng Rust + [egui](https://github.com/emilk/egui).

- Canvas tự do: kéo note bằng thanh tiêu đề, kéo góc dưới phải để resize, kéo nền để di chuyển khung nhìn
- Nhiều trang (tab): tạo, đổi tên (nhấp đúp), xoá (chuột phải), chuyển bằng Ctrl+PgUp/PgDn
- Note có màu, tiêu đề, khoá vị trí (📌), tìm kiếm (note không khớp bị làm mờ)
- **Markdown**: note hiển thị dạng đã render (tiêu đề, **đậm**, *nghiêng*, danh sách, `- [ ]` checkbox tick được, link bấm mở trình duyệt, `code`, trích dẫn). Nhấp đúp hoặc bấm ✏ để sửa, Esc/click ra ngoài để xem lại. Tắt được trong Cài đặt
- **Tách thành việc con**: bôi đen một đoạn trong note → bấm **»** trên thanh note (hoặc Ctrl+Shift+T). Đoạn đó thành note con riêng, được cấp mã ticket theo tên note cha (`RV` → `RV-1`, `RV-2`…), nối mũi tên cha → con, và chỗ vừa cắt để lại neo `» RV-1 tiêu đề` bấm vào là nhảy tới note con (note đó sáng viền trong 1,6 giây). Note con tách tiếp thì nối tiếp mã của cha
- **Sơ đồ**: rê chuột vào note → 4 chấm tròn hiện ở 4 cạnh (trên/dưới/trái/phải), kéo chấm nào cũng được, thả vào note khác để nối mũi tên. Mũi tên mặc định nét đứt và dấu gạch chạy theo hướng mũi tên. Click chọn + Delete để xoá; chuột phải để đặt nhãn, tắt nét đứt, đổi chiều. Mũi tên bám theo khi kéo note; xoá note thì Ctrl+Z khôi phục cả mũi tên. File `.md` xuất ra có mục "Liên kết"
- **Zoom** canvas 30%–300%: Ctrl+cuộn chuột (zoom quanh con trỏ), Ctrl+= / Ctrl+- / Ctrl+0, hoặc nút − % + trên thanh công cụ; mỗi trang nhớ mức zoom riêng
- Tự lưu local sau mỗi lần sửa (debounce 0,8s), ghi file atomic
- Xoá nhầm thì Ctrl+Z hoặc nút "Hoàn tác xoá"
- Đồng bộ Google Drive vào một thư mục cố định: `quick-note-workspace.json` (bản đầy đủ) + mỗi trang một file `.md` để đọc trên điện thoại
- Xuất Markdown ra `~/Documents/QuickNote/`
- **Biểu tượng khay hệ thống**: menu mở bảng ghi chú, tạo ghi chú, đồng bộ Drive, bật/tắt khởi động cùng máy, cài đặt, thoát
- **Khởi động cùng máy**: bật trong ⚙ Cài đặt hoặc menu khay; tạo file `~/.config/autostart/quick-note.desktop`
- **Sticky góc màn hình** (mặc định bật): app thu thành 1 sticky nhỏ ở góc dưới phải màn hình (mặc định màn ngoài cùng bên phải, chọn màn khác trong Cài đặt), luôn nổi trên cùng; rê chuột vào thì trượt ra thành panel full chiều cao (trừ top bar), rộng 1000px; rời chuột thì thu lại

## Sticky góc màn hình trên Wayland

GNOME Wayland không cho app tự đặt vị trí hay "luôn trên cùng", nên ở chế độ này app tự chạy qua **XWayland** (Ubuntu 24.04/26.04 bật sẵn, không cần cấu hình). Chỉnh trong ⚙ Cài đặt → "Sticky góc màn hình": độ rộng, cỡ sticky, thời gian chờ thu gọn, thời lượng hiệu ứng. Bật/tắt chế độ cần mở lại app.

Cửa sổ luôn giữ kích thước panel đầy đủ, nền trong suốt; hiệu ứng mở/đóng chỉ vẽ lại bên trong nên không bị giật. Khi thu gọn, chỉ ô sticky nhận chuột (X11 input shape) — click vào phần trong suốt sẽ xuyên xuống app bên dưới.

- 📌 trên thanh công cụ: giữ panel mở kể cả khi rời chuột
- × hoặc Ctrl+Q: thoát (cửa sổ không có viền)
- `quick-note -w`: mở dạng cửa sổ thường cho lần chạy này

## Biểu tượng khay hệ thống

Dùng chuẩn StatusNotifierItem qua D-Bus. Trên Ubuntu GNOME cần extension **AppIndicator** (Ubuntu bật sẵn). Nếu desktop không có khay, app vẫn chạy bình thường, chỉ không có biểu tượng.

### Nhường chỗ cho app chụp màn hình / quay màn hình

Cửa sổ luôn nổi trên cùng sẽ nằm đè lên lớp phủ toàn màn hình của app khác. Hai lớp bảo vệ:

- Khi đang thu gọn, Quick Note **không nhận bàn phím** (cờ ICCCM `WM_HINTS.input = False`), nên Esc và phím tắt vẫn tới app đang ở trước.
- Ẩn hẳn theo lệnh:

```bash
pkill -USR1 -x quick-note   # ẩn cửa sổ
pkill -USR2 -x quick-note   # hiện lại
```

App chụp màn hình gọi `USR1` trước khi chụp và `USR2` khi đóng lớp chọn vùng, đồng thời **nhắc lại `USR1` mỗi 2 giây** trong suốt thời gian lớp phủ còn mở. Quick Note chỉ hiện lại khi ngừng nhận nhắc 6 giây — nhờ vậy app chụp có chết giữa chừng thì cửa sổ vẫn quay lại, mà phiên chụp dài bao lâu cũng không bị Quick Note nhảy lên che.

## Cài đặt

```bash
sudo apt install build-essential cmake        # C compiler cho thư viện TLS
curl https://sh.rustup.rs -sSf | sh           # nếu chưa có Rust
./scripts/install.sh                          # build release + cài vào ~/.local
```

Sau khi cài, bấm phím Super, gõ "Quick Note" để mở (script ghi đường dẫn tuyệt đối vào file `.desktop` nên GNOME tìm thấy ngay, không cần đăng nhập lại), hoặc mở từ menu ứng dụng hoặc chạy `quick-note`. Chạy lại script để cập nhật (tự tắt bản đang chạy).

Gỡ cài đặt (ghi chú được giữ lại, thêm `--purge` để xoá cả ghi chú — có hỏi xác nhận):

```bash
./scripts/install.sh --uninstall        # hoặc: quick-note --uninstall
```

Tuỳ chọn dòng lệnh (`quick-note -h`):

| Cờ | Chức năng |
|---|---|
| `-h`, `--help` | Trợ giúp |
| `-V`, `--version` | Phiên bản |
| `-w`, `--window` | Mở cửa sổ thường, bỏ qua sticky góc màn hình |
| `--uninstall [--purge]` | Gỡ app (kèm `--purge`: xoá cả dữ liệu) |

Gói `.deb`: `cargo install cargo-deb && cargo deb` → `target/debian/quick-note_*.deb`.

**Phím tắt toàn hệ thống** (mở app nhanh): Settings → Keyboard → View and Customize Shortcuts → Custom Shortcuts → thêm lệnh `quick-note`, gán ví dụ `Super+N`.

## Kết nối Google Drive

### Cách 1: OAuth (tài khoản cá nhân)

App dùng OAuth client của chính bạn (không có server trung gian):

1. Vào [Google Cloud Console](https://console.cloud.google.com/), tạo project.
2. **APIs & Services → Library** → bật **Google Drive API**.
3. **Google Auth Platform → Branding / Audience**: chọn *External*, điền tên app, thêm email của bạn vào *Test users*.
4. **Clients → Create client** → loại **Desktop app** → copy *Client ID* và *Client secret*.
5. Trong Quick Note: ⚙ Cài đặt → dán Client ID / secret → Lưu → bấm **☁ Kết nối Drive** → đăng nhập trên trình duyệt.

Thư mục trên Drive:

| Cấu hình | Quyền | Ghi chú |
|---|---|---|
| Để trống *Folder ID*, đặt *Tên thư mục* | `drive.file` | App tự tạo thư mục ở My Drive, chỉ thấy file nó tạo. Khuyên dùng. |
| Điền *Folder ID* (từ URL `drive.google.com/drive/folders/<ID>`) | `drive` (toàn quyền) | Dùng thư mục có sẵn. |

### Cách 2: Service account (Workspace + Shared Drive)

Không cần đăng nhập trình duyệt, không hết hạn token:

1. Google Cloud Console → **IAM & Admin → Service Accounts** → tạo service account → **Keys → Add key → JSON**, tải file về (để ở nơi riêng tư, ví dụ `~/.config/quick-note/drive-key.json`).
2. Bật **Google Drive API** cho project.
3. Trên Drive, tạo thư mục trong một **Shared Drive**, chia sẻ thư mục đó cho email của service account (dạng `...@....iam.gserviceaccount.com`) với quyền **Content manager**.
4. Trong Quick Note: ⚙ Cài đặt → *Service account key* = đường dẫn file JSON, *Folder ID* = ID thư mục Shared Drive → Lưu → **☁ Kết nối Drive**.

> Bắt buộc dùng Shared Drive: service account không có dung lượng Drive riêng, nên upload vào My Drive sẽ lỗi `storageQuotaExceeded`.
> File JSON key là thông tin đăng nhập vĩnh viễn — nên đặt quyền `chmod 600` và không commit vào git.

> Khi OAuth app ở trạng thái *Testing*, Google thu hồi refresh token sau 7 ngày → phải bấm Kết nối lại.
> Với scope `drive.file` (không nhạy cảm) có thể chuyển app sang *In production* ở mục Audience để tránh việc này.

Đồng bộ: **⬆ Đồng bộ** (hoặc Ctrl+S) đẩy lên; **⬇ Tải về** lấy bản Drive về (hỏi xác nhận, bản local được backup vào `backups/`). Có thể bật tự đồng bộ mỗi N phút trong Cài đặt.

## Phím tắt

| Phím | Chức năng |
|---|---|
| Ctrl+N / nhấp đúp nền | Note mới |
| Ctrl+T | Trang mới |
| Ctrl+S | Lưu + đồng bộ Drive |
| Ctrl+F | Tìm |
| Ctrl+Z | Hoàn tác xoá (khi không gõ chữ) |
| Ctrl+PgUp / PgDn | Chuyển trang |
| Ctrl+= / Ctrl+- / Ctrl+0 | Phóng to / thu nhỏ / về 100% |
| Ctrl+cuộn chuột | Zoom quanh con trỏ |
| Nhấp đúp vào note | Sửa nội dung Markdown |
| Esc | Thôi sửa, xem dạng Markdown |
| Ctrl+Shift+T | Tách phần bôi đen thành note con |
| Delete | Xoá mũi tên đang chọn |
| Ctrl+Q | Thoát |

## Dữ liệu

`~/.config/quick-note/`:

- `workspace.json` — toàn bộ ghi chú
- `config.json` — cài đặt (quyền 600, chứa client secret)
- `drive_token.json` — refresh token (quyền 600)
- `backups/` — bản sao trước mỗi lần thay bằng bản Drive

## Phát triển

```bash
cargo run
cargo test
cargo clippy --all-targets -- -D warnings
QUICK_NOTE_SCREENSHOT=/tmp/shot.ppm cargo run   # build debug: chụp 1 frame rồi thoát
```

```
src/
├── main.rs          khởi tạo cửa sổ, font hệ thống
├── app.rs           state, vòng lặp frame, autosave, xử lý action
├── model.rs         Workspace / Page / Note
├── storage.rs       đọc/ghi JSON atomic, backup, xuất .md
├── config.rs        cài đặt
├── tray.rs          biểu tượng + menu khay hệ thống (D-Bus)
├── autostart.rs     khởi động cùng máy (XDG autostart)
├── signals.rs       SIGUSR1/SIGUSR2 để ẩn/hiện cửa sổ
├── dock.rs          chế độ sticky góc màn hình (XWayland, animation)
├── export.rs        Markdown
├── drive/           OAuth PKCE loopback, service account (JWT RS256), Drive v3 REST, worker
└── ui/              canvas (note), toolbar, dialogs
```
