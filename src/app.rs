use crate::config::{download_dir, Identity, TrustStore};
use crate::runtime::{RuntimeEvent, RuntimeHandle, Peer};
use crate::transfer::IncomingTransfer;
use eframe::egui;
use std::path::PathBuf;

const GREEN: egui::Color32 = egui::Color32::from_rgb(76, 175, 80);
const GREEN_DARK: egui::Color32 = egui::Color32::from_rgb(46, 125, 50);
const BLUE: egui::Color32 = egui::Color32::from_rgb(66, 133, 244);
const RED: egui::Color32 = egui::Color32::from_rgb(220, 80, 80);
const RED_BG: egui::Color32 = egui::Color32::from_rgb(60, 24, 24);
const AMBER: egui::Color32 = egui::Color32::from_rgb(255, 200, 80);
const MUTED: egui::Color32 = egui::Color32::from_rgb(160, 165, 175);

#[derive(Clone, Copy, PartialEq, Eq)]
enum StatusKind {
    Ready,
    Info,
    Success,
    Error,
}

pub struct RelayApp {
    identity: Identity,
    runtime: RuntimeHandle,
    peers: Vec<Peer>,
    selected_peer: Option<usize>,
    pending_files: Vec<PathBuf>,
    status: String,
    status_kind: StatusKind,
    logs: Vec<String>,
    pending_incoming: Option<PendingIncoming>,
    transfer_progress: Option<f32>,
    transfer_label: String,
}

struct PendingIncoming {
    remote_name: String,
    pairing_code: String,
    needs_pairing: bool,
    transfer: Option<IncomingTransfer>,
}

impl RelayApp {
    pub fn new(cc: &eframe::CreationContext<'_>, identity: Identity, runtime: RuntimeHandle) -> Self {
        apply_theme(&cc.egui_ctx);
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
            status_kind: StatusKind::Ready,
            logs: Vec::new(),
            pending_incoming: None,
            transfer_progress: None,
            transfer_label: String::new(),
        }
    }

    fn set_status(&mut self, kind: StatusKind, message: impl Into<String>) {
        self.status_kind = kind;
        self.status = message.into();
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
                RuntimeEvent::IncomingConnection {
                    remote_name,
                    pairing_code,
                    needs_pairing,
                    transfer,
                } => {
                    self.set_status(
                        StatusKind::Info,
                        format!("Incoming connection from {remote_name}"),
                    );
                    self.pending_incoming = Some(PendingIncoming {
                        remote_name,
                        pairing_code,
                        needs_pairing,
                        transfer: Some(transfer),
                    });
                }
                RuntimeEvent::TransferProgress { fraction, label } => {
                    self.transfer_progress = Some(fraction.clamp(0.0, 1.0));
                    self.transfer_label = label;
                }
                RuntimeEvent::SendFinished { ok, message } => {
                    self.transfer_progress = None;
                    self.transfer_label.clear();
                    self.set_status(
                        if ok { StatusKind::Success } else { StatusKind::Error },
                        message.clone(),
                    );
                    self.log(ok, message);
                    if ok {
                        self.pending_files.clear();
                    }
                }
                RuntimeEvent::ReceiveFinished { ok, message } => {
                    self.transfer_progress = None;
                    self.transfer_label.clear();
                    self.set_status(
                        if ok { StatusKind::Success } else { StatusKind::Error },
                        message.clone(),
                    );
                    self.log(ok, message);
                    self.pending_incoming = None;
                }
                RuntimeEvent::Log(message) => {
                    let is_error = message.to_lowercase().contains("error")
                        || message.to_lowercase().contains("failed");
                    if is_error {
                        self.set_status(StatusKind::Error, message.clone());
                    }
                    self.log(false, message);
                }
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

    fn draw_error_banner(&self, ui: &mut egui::Ui) {
        if self.status_kind != StatusKind::Error {
            return;
        }
        egui::Frame::new()
            .fill(RED_BG)
            .stroke(egui::Stroke::new(1.5, RED))
            .inner_margin(egui::Margin::symmetric(14, 10))
            .corner_radius(6.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("⚠")
                            .size(20.0)
                            .color(RED),
                    );
                    ui.label(
                        egui::RichText::new(&self.status)
                            .size(16.0)
                            .color(egui::Color32::from_rgb(255, 190, 190)),
                    );
                });
            });
        ui.add_space(8.0);
    }

    fn draw_progress(&self, ui: &mut egui::Ui) {
        if let Some(fraction) = self.transfer_progress {
            ui.add_space(4.0);
            let label = if self.transfer_label.is_empty() {
                format!("{:.0}%", fraction * 100.0)
            } else {
                self.transfer_label.clone()
            };
            ui.add(
                egui::ProgressBar::new(fraction)
                    .fill(GREEN)
                    .animate(true)
                    .text(label),
            );
            ui.add_space(8.0);
        }
    }

    fn draw_header(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("NewTowerRelay").strong().size(26.0));
            ui.separator();
            ui.label(
                egui::RichText::new(format!("This device: {}", self.identity.name))
                    .size(16.0),
            );
        });

        let status_color = match self.status_kind {
            StatusKind::Ready => MUTED,
            StatusKind::Info => egui::Color32::from_rgb(140, 190, 255),
            StatusKind::Success => GREEN,
            StatusKind::Error => RED,
        };
        ui.label(egui::RichText::new(&self.status).size(15.0).color(status_color));
    }

    fn draw_peers(&mut self, ui: &mut egui::Ui) {
        section_heading(ui, "Nearby devices");
        ui.add_space(6.0);

        if self.peers.is_empty() {
            ui.label(
                egui::RichText::new("Searching on your network…")
                    .size(15.0)
                    .color(MUTED),
            );
            ui.label(
                egui::RichText::new("Make sure both computers run NewTowerRelay on the same Wi‑Fi.")
                    .size(14.0)
                    .color(MUTED),
            );
            return;
        }

        egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
            for (idx, peer) in self.peers.iter().enumerate() {
                let selected = self.selected_peer == Some(idx);
                let fill = if selected {
                    egui::Color32::from_rgb(40, 70, 110)
                } else {
                    egui::Color32::from_rgb(32, 36, 44)
                };
                let stroke = if selected {
                    egui::Stroke::new(2.0, BLUE)
                } else {
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(55, 60, 70))
                };
                let response = ui.add(
                    egui::Button::new(egui::RichText::new(&peer.name).size(16.0))
                        .fill(fill)
                        .stroke(stroke)
                        .min_size(egui::vec2(ui.available_width(), 40.0)),
                );
                if response.clicked() {
                    self.selected_peer = Some(idx);
                }
            }
        });
    }

    fn draw_send(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        section_heading(ui, "Send files");
        ui.add_space(6.0);

        if secondary_button(ui, "Choose files…").clicked() {
            if let Some(paths) = rfd::FileDialog::new().pick_files() {
                self.pending_files = paths;
                self.set_status(StatusKind::Info, "Files selected — pick a peer and send.");
            }
        }

        if !self.pending_files.is_empty() {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("{} file(s) selected:", self.pending_files.len()))
                    .size(15.0),
            );
            for path in &self.pending_files {
                ui.label(
                    egui::RichText::new(format!("  • {}", path.display()))
                        .size(14.0)
                        .color(MUTED),
                );
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
                self.set_status(StatusKind::Info, "Files dropped — pick a peer and send.");
            }
        }

        ui.add_space(12.0);
        let can_send = self.selected_peer.is_some() && !self.pending_files.is_empty();
        let sending = self.transfer_progress.is_some();

        if !can_send && !sending {
            ui.label(
                egui::RichText::new("Select a nearby device and choose files to enable sending.")
                    .size(14.0)
                    .color(MUTED),
            );
            ui.add_space(4.0);
        }

        if primary_button(ui, "Send encrypted", can_send && !sending).clicked() {
            if let Some(idx) = self.selected_peer {
                let peer = self.peers[idx].clone();
                let paths = self.pending_files.clone();
                self.transfer_progress = Some(0.01);
                self.transfer_label = format!("Sending to {}…", peer.name);
                self.set_status(StatusKind::Info, format!("Sending to {}…", peer.name));
                self.runtime.send(peer, paths);
            }
        }
    }

    fn draw_incoming(&mut self, ui: &mut egui::Ui) {
        let mut accept = false;
        let mut decline = false;

        if let Some(pending) = self.pending_incoming.as_ref() {
            ui.separator();
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Incoming connection")
                    .strong()
                    .size(18.0)
                    .color(AMBER),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("From: {}", pending.remote_name))
                    .size(16.0),
            );
            ui.label(
                egui::RichText::new("Accept to receive files. The file list arrives after you accept.")
                    .size(14.0)
                    .color(MUTED),
            );

            if pending.needs_pairing {
                ui.add_space(8.0);
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(35, 38, 48))
                    .stroke(egui::Stroke::new(1.0, AMBER))
                    .inner_margin(12.0)
                    .corner_radius(6.0)
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new("Pairing code — verify on both devices:")
                                .size(14.0),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(&pending.pairing_code)
                                .strong()
                                .monospace()
                                .size(28.0)
                                .color(AMBER),
                        );
                    });
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if accept_button(ui, "Accept").clicked() {
                    accept = true;
                }
                ui.add_space(8.0);
                if danger_button(ui, "Decline").clicked() {
                    decline = true;
                }
            });
        }

        if accept {
            if let Some(pending) = self.pending_incoming.as_mut() {
                if let Some(transfer) = pending.transfer.take() {
                    self.transfer_progress = Some(0.01);
                    self.transfer_label = "Receiving…".to_string();
                    self.set_status(StatusKind::Info, "Receiving…".to_string());
                    self.runtime.accept(transfer);
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
            self.set_status(StatusKind::Info, "Transfer declined".to_string());
        }
    }

    fn draw_log(&self, ui: &mut egui::Ui) {
        section_heading(ui, "Activity");
        egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
            if self.logs.is_empty() {
                ui.label(
                    egui::RichText::new("No activity yet.")
                        .size(14.0)
                        .color(MUTED),
                );
            }
            for line in &self.logs {
                let color = if line.starts_with('✓') {
                    GREEN
                } else if line.to_lowercase().contains("failed")
                    || line.to_lowercase().contains("error")
                {
                    RED
                } else {
                    MUTED
                };
                ui.label(egui::RichText::new(line).size(14.0).color(color));
            }
        });
    }
}

impl eframe::App for RelayApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events();
        ctx.request_repaint_after(std::time::Duration::from_millis(200));

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(10.0);
            self.draw_error_banner(ui);
            self.draw_header(ui);
            self.draw_progress(ui);
            ui.add_space(12.0);

            ui.columns(2, |cols| {
                cols[0].vertical(|ui| {
                    self.draw_peers(ui);
                    ui.add_space(16.0);
                    self.draw_send(ui, ctx);
                });
                cols[1].vertical(|ui| {
                    self.draw_incoming(ui);
                    ui.add_space(16.0);
                    self.draw_log(ui);
                });
            });
        });
    }
}

fn section_heading(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).strong().size(18.0));
}

fn primary_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> egui::Response {
    let fill = if enabled { GREEN_DARK } else { egui::Color32::from_rgb(55, 58, 62) };
    let text_color = if enabled {
        egui::Color32::WHITE
    } else {
        egui::Color32::from_rgb(120, 125, 130)
    };
    ui.add_enabled(
        enabled,
        egui::Button::new(egui::RichText::new(label).strong().size(17.0).color(text_color))
            .fill(fill)
            .stroke(egui::Stroke::new(
                1.0,
                if enabled {
                    GREEN
                } else {
                    egui::Color32::from_rgb(70, 74, 80)
                },
            ))
            .min_size(egui::vec2(220.0, 46.0)),
    )
}

fn secondary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).size(15.0))
            .fill(egui::Color32::from_rgb(40, 44, 52))
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(80, 86, 96)))
            .min_size(egui::vec2(140.0, 36.0)),
    )
}

fn accept_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).strong().size(16.0).color(egui::Color32::WHITE))
            .fill(GREEN_DARK)
            .stroke(egui::Stroke::new(1.0, GREEN))
            .min_size(egui::vec2(120.0, 40.0)),
    )
}

fn danger_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).size(16.0))
            .fill(egui::Color32::from_rgb(70, 30, 30))
            .stroke(egui::Stroke::new(1.0, RED))
            .min_size(egui::vec2(120.0, 40.0)),
    )
}

fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = egui::Color32::from_rgb(22, 24, 28);
    visuals.window_fill = egui::Color32::from_rgb(16, 18, 22);
    visuals.selection.bg_fill = egui::Color32::from_rgb(60, 120, 200);
    visuals.widgets.noninteractive.fg_stroke.color = egui::Color32::from_rgb(210, 215, 225);
    visuals.widgets.inactive.fg_stroke.color = egui::Color32::from_rgb(210, 215, 225);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::new(16.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::new(16.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::new(22.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        egui::FontId::new(20.0, egui::FontFamily::Monospace),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        egui::FontId::new(14.0, egui::FontFamily::Proportional),
    );
    style.spacing.button_padding = egui::vec2(14.0, 10.0);
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.indent = 18.0;
    ctx.set_style(style);
}

pub fn run(identity: Identity, trust: TrustStore) -> eframe::Result<()> {
    let runtime = crate::runtime::spawn(identity.clone(), trust)
        .map_err(|e| eframe::Error::AppCreation(format!("{e}").into()))?;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 620.0])
            .with_min_inner_size([700.0, 480.0])
            .with_title("NewTowerRelay"),
        ..Default::default()
    };

    eframe::run_native(
        "NewTowerRelay",
        options,
        Box::new(move |cc| Ok(Box::new(RelayApp::new(cc, identity, runtime)))),
    )
}
