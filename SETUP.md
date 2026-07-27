# NewTowerRelay — Setup (Executable)

Download the app for your platform, run it, and share files on your local network. **No Rust, no terminal build steps required.**

Both computers must run NewTowerRelay on the **same Wi‑Fi or LAN**.

---

## Downloads

Each release includes **four executables** — pick the one for your system:

| Platform | File | Who should use it |
|----------|------|-------------------|
| **macOS** (recommended) | `NewTowerRelay-*-macOS-Universal` | Any Mac — Intel and Apple Silicon |
| **macOS** (Intel only) | `NewTowerRelay-*-macOS-Intel` | Older Intel Macs |
| **macOS** (Apple Silicon) | `NewTowerRelay-*-macOS-AppleSilicon` | M1/M2/M3/M4 Macs |
| **Linux** (recommended) | `ntrelay_*_amd64.deb` | Mint, Ubuntu, Debian — double-click or `apt install` |
| **Linux** (portable) | `NewTowerRelay-*-Linux-x86_64` | Run without installing |
| **Windows** | `NewTowerRelay-*-Windows-x86_64.exe` | Windows 10/11 (64-bit) |

Published on the project **Releases** page when you tag a version (e.g. `v0.1.0`).  
Maintainers can also build locally with `./scripts/build-release.sh` → output in `dist/`.

---

## macOS

1. Download `NewTowerRelay-*-macOS-Universal` (works on Intel and M-series Macs).
2. Move it to **Applications** or anywhere you like.
3. **First launch:** right-click the file → **Open** → **Open** again.  
   (macOS may block unsigned apps on first run — this bypasses Gatekeeper once.)
4. Allow **Local Network** access if macOS asks — required to find other devices.

**Run from Terminal (optional):**

```bash
chmod +x ~/Downloads/NewTowerRelay-*-macOS-Universal
~/Downloads/NewTowerRelay-*-macOS-Universal
```

---

## Linux

**Recommended — `.deb` installer** (Mint, Ubuntu, Debian):

1. Download `ntrelay_*_amd64.deb` from Releases.
2. Double-click the file, or run:

```bash
sudo apt install ./ntrelay_0.1.0_amd64.deb
```

This installs **NTRelay** to your app menu, puts `ntrelay` on your PATH, and sets up Avahi for device discovery.

**Standalone binary** (no install):

1. Download `NewTowerRelay-*-Linux-x86_64`.
2. Make it executable and run:

```bash
chmod +x NewTowerRelay-*-Linux-x86_64
./NewTowerRelay-*-Linux-x86_64
```

3. **mDNS:** most desktops already run Avahi. If peers don’t show up:

```bash
sudo systemctl enable --now avahi-daemon
```

4. Install **OpenGL** / graphics libs only if the app fails to start (varies by distro):

```bash
# Debian / Ubuntu
sudo apt install libxcb1 libxkbcommon0 libssl3

# Fedora
sudo dnf install libxcb libxkbcommon openssl-libs
```

---

## Windows

1. Download `NewTowerRelay-*-Windows-x86_64.exe`.
2. Double-click to run.
3. If **Windows Defender Firewall** prompts you, allow access on **Private networks** (not public hotspots unless you trust everyone on it).
4. Keep the `.exe` anywhere — Desktop, Downloads, or `C:\Program Files\NewTowerRelay\`.

**No installer required** — it’s a single portable executable.

---

## Using the app

1. Open **NewTowerRelay** on **both** computers.
2. Wait a few seconds — the other machine appears under **Nearby devices**.
3. **Send:** choose files (or drag-and-drop), pick the peer, click **Send encrypted**.
4. **Receive:** check the **pairing code** matches on first contact, then click **Accept**.

### Where received files go

| Platform | Folder |
|----------|--------|
| macOS | `~/Library/Application Support/com.NewTower/NewTowerRelay/Downloads/` |
| Linux | `~/.local/share/newtowerrelay/Downloads/` |
| Windows | `%LOCALAPPDATA%\NewTower\NewTowerRelay\data\Downloads\` |

---

## Troubleshooting

| Problem | Fix |
|---------|-----|
| **Can’t see the other computer** | Same Wi‑Fi/LAN; disable guest network isolation; allow firewall/local network |
| **macOS “can’t be opened”** | Right-click → Open (first time only) |
| **Linux: app won’t start** | Install libxcb / libxkbcommon (see above); run from terminal to see errors |
| **Windows: no peers** | Allow through firewall on private network |
| **Transfer stuck** | Receiver must click **Accept**; keep both apps open |

---

## Building executables (maintainers only)

If you need to *create* the downloadable files (not required for normal users):

```bash
# Requires Rust — see scripts/README.md
./scripts/build-release.sh
```

Output lands in `dist/`. Ship those files to users — they don’t need Rust installed.
