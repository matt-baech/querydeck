# litedb

A lightweight, native desktop SQL client for **SQL Server** and **PostgreSQL**, written in Rust.

litedb aims to cover the everyday 90% of what you'd open SQL Server Management Studio or Azure Data Studio for (browse a server, write queries, look at results) in a fast, single-binary app that starts instantly.

## Features

- **Object Explorer** in the style of SSMS: server → databases → tables, views, stored procedures, functions and triggers, with columns (type, nullability, primary key) under each table. Everything loads lazily as you expand it.
- **Right-click actions**: *Select Top 1000 Rows*, *Script* (view/procedure/function/trigger source), *New Query* in a given database, *Refresh*.
- **Query tabs**, each with its own connection, so a long query never blocks the tree and a `USE` in one tab doesn't affect another.
- **SQL editor** with syntax highlighting. Runs the whole script or just the selection. SQL Server scripts can be split into batches with `GO`.
- **Results grid** with multiple result sets, paging, right-aligned numbers and distinct `NULL`s, plus a Messages tab with row counts, errors and timings.
- **Cancel** a running query on the server.
- **Saved connections**, with passwords stored in your OS keyring (KWallet / GNOME Keyring / Windows Credential Manager), never in plain text.
- **Light and dark themes** (or follow the system), muted colours, resizable panes.
- **Minimize to tray** on close (Windows, macOS, and Linux desktops with a StatusNotifierItem tray, such as KDE Plasma).

## Installation

There are no prebuilt binaries yet, so build it from source. It's one command once Rust is installed.

### 1. Install Rust

Install Rust 1.88 or newer:

- **Arch / CachyOS / Manjaro:** `sudo pacman -S rust`
- **Anywhere else:** use [rustup](https://rustup.rs): `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`

### 2. System requirements

- **Graphics:** a GPU driver with Vulkan, Metal or DirectX 12 support. litedb renders with `wgpu` and falls back to a software renderer if needed.
- **Linux:** a Wayland or X11 desktop. To remember passwords, a Secret Service provider (KWallet on KDE, GNOME Keyring on GNOME). For minimize-to-tray, a StatusNotifierItem tray (built into KDE Plasma; GNOME needs the *AppIndicator* extension).
- **Windows / macOS:** should build and run, but haven't been tested on real machines yet. The tray icon sits in the notification area (Windows) or the menu bar (macOS); left-click shows or hides the window, right-click opens its menu.

### 3. Build and install

```bash
git clone https://github.com/matt-baech/litedb.git
cd litedb
cargo install --path .
```

This builds an optimized binary and copies it to `~/.cargo/bin/litedb`. Make sure `~/.cargo/bin` is on your `PATH`, then start it with:

```bash
litedb
```

To build without installing, run `cargo build --release`. The binary is then at `target/release/litedb` (about 30 MB).

### Optional: add it to your application menu (Linux)

```bash
cat > ~/.local/share/applications/litedb.desktop <<'EOF'
[Desktop Entry]
Name=litedb
Comment=Lightweight SQL client
Exec=litedb
Icon=server-database
Terminal=false
Type=Application
Categories=Development;Database;
StartupWMClass=litedb
EOF
```

### Uninstall

```bash
cargo uninstall litedb
rm -rf ~/.config/litedb
```

## Usage

### Connecting

The **Connect to Server** dialog opens on start (or from the plug icon in the Object Explorer):

1. Pick **SQL Server** or **PostgreSQL**, and fill in host, port, user, password and database.
2. For a local SQL Server (e.g. Docker), leave **Trust server certificate** ticked.
3. Click **Save** to keep the connection (tick *Save password in keyring* to remember the password), then **Connect**.

No server handy? Start SQL Server in Docker:

```bash
docker run -d --name mssql -e ACCEPT_EULA=Y -e 'MSSQL_SA_PASSWORD=YourStrong!Passw0rd' -p 1433:1433 mcr.microsoft.com/mssql/server:2022-latest
```

Then connect to `localhost`, port `1433`, user `sa`. [`examples/persons.sql`](examples/persons.sql) creates a sample `Persons` table with 1000 rows of mock data.

### Working with queries

- Expand the tree in the **Object Explorer** and **right-click** any node for actions.
- Write SQL in a query tab and press **F5**. If text is selected, only the selection runs.
- Switch between **Results** and **Messages** below the editor. Multiple result sets get numbered buttons.
- Drag the divider lines to resize the explorer and the editor/results split.

### Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `F5` / `Ctrl+Enter` | Run script or selection |
| `Esc` | Cancel running query / close dialog |
| `Ctrl+N` | New query tab |
| `Ctrl+,` | Settings |

### Settings

Open with the gear icon in the status bar or `Ctrl+,`: theme, editor font size, rows per page, and minimize to tray on close.

Settings and saved connections are stored as JSON in your config directory:

| OS | Location |
|---|---|
| Linux | `~/.config/litedb/` |
| Windows | `%APPDATA%\litedb\config\` |
| macOS | `~/Library/Application Support/litedb/` |

Passwords are never written there. They live in the OS keyring under the service name `litedb`.

## Limitations

- PostgreSQL connections don't use TLS yet (fine for local servers and Docker, not for hosted databases that require it).
- PostgreSQL's Object Explorer shows only the database you're connected to.
- SQL Server: SQL logins only (no Windows Integrated Authentication). Only table triggers are listed.
- A query keeps at most 100,000 rows per result set.
- The editor has no line numbers, and grid cells can't be selected or copied yet.
- macOS: while litedb is in the tray, clicking its Dock icon doesn't bring the window back; use the menu-bar icon.

## Built with

[Iced](https://iced.rs) (UI) · [tiberius](https://github.com/prisma/tiberius) (SQL Server) · [tokio-postgres](https://github.com/sfackler/rust-postgres) (PostgreSQL) · [keyring](https://github.com/open-source-cooperative/keyring-rs) · [ksni](https://github.com/iovxw/ksni) and [tray-icon](https://github.com/tauri-apps/tray-icon) (tray) · [iced_aw](https://github.com/iced-rs/iced_aw) (context menus)

Bundled fonts: [Inter](https://rsms.me/inter/) (SIL Open Font License 1.1), [Hack](https://sourcefoundry.org/hack/) (MIT), and [Codicons](https://github.com/microsoft/vscode-codicons) (CC BY 4.0). Their licence texts are in [`assets/fonts`](assets/fonts).
