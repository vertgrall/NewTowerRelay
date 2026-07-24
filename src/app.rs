use crate::config::{download_dir, Identity, TrustStore};
use crate::protocol::FileMeta;
use crate::runtime::{RuntimeEvent, RuntimeHandle, Peer};
use crate::transfer::IncomingTransfer;
use eframe::egui;
use std::path::PathBuf;

pub struct RelayApp {
    identity: Identity,
    runtime: RuntimeHandle,
    peers: Vec<Peer>,
    selected_peer: Option<usize>,
    pending_files: Vec<PathBuf>,
    status: String,
    logs: Vec<String>,
    pending_incoming: Option<PendingIncoming>,
}

struct PendingIncoming {
    remote_name: String,
    files: Vec<FileMeta>,
    pairing_code: String,
    needs_pairing: bool,
    transfer: Option<IncomingTransfer>,
}

impl RelayApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, identity: Identity, runtime: RuntimeHandle) -> Self {
        apply_theme(&_cc.egui_ctx);
        let download = download_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "~/Downloads".to_string());
        Self {
            identity,
            runtime,
            peers: Vec::new(),
            selected_peer: None,
            pending_files: Vec::new(),
            status: format!("Ready — saves to {download}"),
            logs: Vec::new(),
            pending_incoming: None,
        }
    }

    fn poll_events(&mut self) {
        while let Ok(event) = self.runtime.events.try_recv() {
            match event {
                RuntimeEvent::PeersUpdated(peers) => {
                    self.peers = peers;
                    if self.selected_peer >= Some(self.peers.len()) {
                        self.selected_peer = None;
                    }
                }
                RuntimeEvent::IncomingOffer {
                    remote_name,
                    files,
                    pairing_code,
                    needs_pairing,
                    transfer,
                } => {
                    self.status = format!("Incoming transfer from {remote_name}");
                    self.pending_incoming = Some(PendingIncoming {
                        remote_name,
                        files,
                        pairing_code,
                        needs_pairing,
                        transfer: Some(transfer),
                    });
                }
                RuntimeEvent::SendFinished { ok, message } => {
                    self.status = message.clone();
                    self.log(ok, message);
                    if ok {
                        self.pending_files.clear();
                    }
                }
                RuntimeEvent::ReceiveFinished { ok, message } => {
                    self.status = message.clone();
                    self.log(ok, message);
                    self.pending_incoming = None;
                }
                RuntimeEvent::Log(message) => self.log(false, message),
            }
        }
    }

    fn log(&mut self, ok: bool, message: String) {
        let prefix = if ok { "✓" } else { "·" };
        self.logs.push(format!("{prefix} {message}"));
        if self.logs.len() > 100 {
            self.logs.remove(0);
        }
    }

    fn draw_header(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("NewTowerRelay").strong().size(18.0));
            ui.separator();
            ui.label(format!("This device: {}", self.identity.name));
        });
        ui.label(egui::RichText::new(&self.status).weak());
    }

    fn draw_peers(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Nearby devices").strong());
        ui.add_space(4.0);
        if self.peers.is_empty() {
            ui.label(egui::RichText::new("Searching on your network…").weak());
            ui.label(egui::RichText::new("Make sure both computers run NewTowerRelay on the same Wi‑Fi.").small().weak());
            return;
        }
        egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
            for (idx, peer) in self.peers.iter().enumerate() {
                if ui
                    .selectable_label(self.selected_peer == Some(idx), &peer.name)
                    .clicked()
                {
                    self.selected_peer = Some(idx);
                }
            }
        });
    }

    fn draw_send(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(egui::RichText::new("Send files").strong());
        ui.add_space(4.0);

        if ui.button("Choose files…").clicked() {
            if let Some(paths) = rfd::FileDialog::new().pick_files() {
                self.pending_files = paths;
            }
        }

        if !self.pending_files.is_empty() {
            ui.label(format!("{} file(s) selected:", self.pending_files.len()));
            for path in &self.pending_files {
                ui.label(format!("  • {}", path.display()));
            }
        }

        if !ctx.input(|i| i.raw.dropped_files.is_empty()) {
            let dropped: Vec<PathBuf> = ctx
                .input(|i| {
                    i.raw
                        .dropped_files
                        .iter()
                        .filter_map(|f| f.path.clone())
                        .collect()
                });
            if !dropped.is_empty() {
                self.pending_files = dropped;
            }
        }

        ui.add_space(8.0);
        let can_send = self.selected_peer.is_some() && !self.pending_files.is_empty();
        ui.add_enabled_ui(can_send, |ui| {
            if ui.button("Send encrypted").clicked() {
                if let Some(idx) = self.selected_peer {
                    let peer = self.peers[idx].clone();
                    let paths = self.pending_files.clone();
                    self.status = format!("Sending to {}…", peer.name);
                    self.runtime.send(peer, paths);
                }
            }
        });
    }

    fn draw_incoming(&mut self, ui: &mut egui::Ui) {
        let mut accept = false;
        let mut decline = false;

        if let Some(pending) = self.pending_incoming.as_ref() {
            ui.separator();
            ui.label(
                egui::RichText::new("Incoming transfer")
                    .strong()
                    .color(egui::Color32::from_rgb(255, 200, 80)),
            );
            ui.label(format!("From: {}", pending.remote_name));
            for file in &pending.files {
                ui.label(format!("  • {} ({})", file.name, format_size(file.size)));
            }
            if pending.needs_pairing {
                ui.horizontal(|ui| {
                    ui.label("Pairing code:");
                    ui.label(
                        egui::RichText::new(&pending.pairing_code)
                            .strong()
                            .monospace(),
                    );
                });
                ui.label(
                    egui::RichText::new("Verify this code matches on the sender before accepting.")
                        .small()
                        .weak(),
                );
            }
            ui.horizontal(|ui| {
                if ui.button("Accept").clicked() {
                    accept = true;
                }
                if ui.button("Decline").clicked() {
                    decline = true;
                }
            });
        }

        if accept {
            if let Some(pending) = self.pending_incoming.as_mut() {
                if let Some(transfer) = pending.transfer.take() {
                    self.runtime.accept(transfer);
                    self.status = "Receiving…".to_string();
                }
            }
        }
        if decline {
            if let Some(pending) = self.pending_incoming.as_mut() {
                if let Some(transfer) = pending.transfer.take() {
                    self.runtime.decline(transfer);
                }
            }
            self.pending_incoming = None;
        }
    }

    fn draw_log(&self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Activity").strong());
        egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
            if self.logs.is_empty() {
                ui.label(egui::RichText::new("No activity yet.").weak());
            }
            for line in &self.logs {
                ui.label(egui::RichText::new(line).small());
            }
        });
    }
}

impl eframe::App for RelayApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events();
        ctx.request_repaint_after(std::time::Duration::from_millis(500));

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            self.draw_header(ui);
            ui.add_space(12.0);

            ui.columns(2, |cols| {
                self.draw_peers(&mut cols[0]);
                cols[0].add_space(12.0);
                self.draw_send(&mut cols[0], ctx);
                cols[1].vertical(|ui| {
                    self.draw_incoming(ui);
                    ui.add_space(12.0);
                    self.draw_log(ui);
                });
            });
        });
    }
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = egui::Color32::from_rgb(22, 24, 28);
    visuals.window_fill = egui::Color32::from_rgb(16, 18, 22);
    visuals.selection.bg_fill = egui::Color32::from_rgb(60, 120, 200);
    ctx.set_visuals(visuals);
}

pub fn run(identity: Identity, trust: TrustStore) -> eframe::Result<()> {
    let runtime = crate::runtime::spawn(identity.clone(), trust)
        .map_err(|e| eframe::Error::AppCreation(format!("{e}").into()))?;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([820.0, 560.0])
            .with_min_inner_size([640.0, 420.0])
            .with_title("NewTowerRelay"),
        ..Default::default()
    };

    eframe::run_native(
        "NewTowerRelay",
        options,
        Box::new(move |cc| Ok(Box::new(RelayApp::new(cc, identity, runtime)))),
    )
}
