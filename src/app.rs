use crate::config::{download_dir, Identity, TrustStore};
use crate::runtime::{Peer, RuntimeEvent, RuntimeHandle};
use crate::peer_registry::PeerPresence;
use crate::transfer::IncomingTransfer;
use eframe::egui;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const BTN_BLUE: egui::Color32 = egui::Color32::from_rgb(37, 99, 235);
const BTN_BLUE_DARK: egui::Color32 = egui::Color32::from_rgb(29, 78, 216);
const BTN_BLUE_HOVER: egui::Color32 = egui::Color32::from_rgb(59, 130, 246);
const BTN_BLUE_DISABLED: egui::Color32 = egui::Color32::from_rgb(147, 170, 220);
const BTN_STROKE: egui::Color32 = egui::Color32::from_rgb(30, 64, 175);
const BTN_GREEN: egui::Color32 = egui::Color32::from_rgb(34, 197, 94);
const BTN_GREEN_HOVER: egui::Color32 = egui::Color32::from_rgb(52, 211, 122);
const BTN_GREEN_DISABLED: egui::Color32 = egui::Color32::from_rgb(134, 200, 160);
const BTN_GREEN_STROKE: egui::Color32 = egui::Color32::from_rgb(21, 128, 61);
const GREEN: egui::Color32 = egui::Color32::from_rgb(22, 163, 74);
const RED: egui::Color32 = egui::Color32::from_rgb(220, 38, 38);
const AMBER: egui::Color32 = egui::Color32::from_rgb(180, 120, 0);
const INTER_SEMIBOLD: &str = "inter-semibold";
const BTN_RADIUS: f32 = 12.0;
const CARD_RADIUS: f32 = 12.0;

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
    selected_device_id: Option<String>,
    dismissed_new_peers: HashSet<String>,
    pending_files: Vec<PathBuf>,
    status: String,
    status_kind: StatusKind,
    logs: Vec<String>,
    pending_incoming: Option<PendingIncoming>,
    transfer_progress: Option<f32>,
    transfer_label: String,
    last_theme: egui::Theme,
    pending_send_pairing: Option<PendingSendPairing>,
    show_peers: bool,
    show_send: bool,
    show_activity: bool,
    show_open_downloads: bool,
}

#[derive(Clone)]
struct PendingSendPairing {
    peer_name: String,
    pairing_code: String,
}

struct PendingIncoming {
    remote_name: String,
    pairing_code: String,
    needs_pairing: bool,
    transfer: Option<IncomingTransfer>,
}

impl RelayApp {
    pub fn new(cc: &eframe::CreationContext<'_>, identity: Identity, runtime: RuntimeHandle) -> Self {
        configure_inter_fonts(&cc.egui_ctx);
        apply_theme(&cc.egui_ctx, cc.egui_ctx.theme());
        let download = download_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "~/Downloads".to_string());
        Self {
            identity,
            runtime,
            peers: Vec::new(),
            selected_device_id: None,
            dismissed_new_peers: HashSet::new(),
            pending_files: Vec::new(),
            status: format!("Ready — saves to {download}"),
            status_kind: StatusKind::Ready,
            logs: Vec::new(),
            pending_incoming: None,
            transfer_progress: None,
            transfer_label: String::new(),
            last_theme: cc.egui_ctx.theme(),
            pending_send_pairing: None,
            show_peers: true,
            show_send: true,
            show_activity: true,
            show_open_downloads: false,
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
                    if let Some(id) = &self.selected_device_id {
                        if !peers.iter().any(|p| &p.device_id == id) {
                            self.selected_device_id = None;
                        }
                    }
                    self.peers = peers;
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
                RuntimeEvent::SendPairingConfirm {
                    peer_name,
                    pairing_code,
                } => {
                    self.transfer_progress = Some(0.06);
                    self.transfer_label = format!("Confirm pairing with {peer_name}");
                    self.set_status(
                        StatusKind::Info,
                        format!("Confirm pairing code with {peer_name}"),
                    );
                    self.pending_send_pairing = Some(PendingSendPairing {
                        peer_name,
                        pairing_code,
                    });
                }
                RuntimeEvent::TransferProgress { fraction, label } => {
                    self.transfer_progress = Some(fraction.clamp(0.0, 1.0));
                    self.transfer_label = label;
                }
                RuntimeEvent::TransferCancelled { message } => {
                    self.transfer_progress = None;
                    self.transfer_label.clear();
                    self.pending_send_pairing = None;
                    self.pending_incoming = None;
                    self.set_status(StatusKind::Info, message.clone());
                    self.log(false, message);
                }
                RuntimeEvent::SendFinished { ok, message } => {
                    self.transfer_progress = None;
                    self.transfer_label.clear();
                    self.pending_send_pairing = None;
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
                    self.show_open_downloads = ok;
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
        let dark = ui.visuals().dark_mode;
        let (fill, stroke, text) = if dark {
            (
                egui::Color32::from_rgba_unmultiplied(80, 28, 28, 220),
                RED,
                egui::Color32::from_rgb(254, 202, 202),
            )
        } else {
            (
                egui::Color32::from_rgba_unmultiplied(254, 226, 226, 230),
                egui::Color32::from_rgb(185, 28, 28),
                egui::Color32::from_rgb(127, 29, 29),
            )
        };
        egui::Frame::new()
            .fill(fill)
            .stroke(egui::Stroke::new(1.5_f32, stroke))
            .inner_margin(egui::Margin::symmetric(12, 8))
            .corner_radius(CARD_RADIUS)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("⚠").size(18.0).color(stroke));
                    ui.label(egui::RichText::new(&self.status).size(14.0).color(text));
                });
            });
        ui.add_space(6.0);
    }

    fn draw_progress(&mut self, ui: &mut egui::Ui) {
        if let Some(fraction) = self.transfer_progress {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let label = if self.transfer_label.is_empty() {
                    format!("{:.0}%", fraction * 100.0)
                } else {
                    self.transfer_label.clone()
                };
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .fill(BTN_GREEN)
                        .animate(true)
                        .text(label)
                        .corner_radius(CARD_RADIUS),
                );
                if blue_button(ui, "Cancel", true, false, egui::vec2(72.0, 36.0), 13.0).clicked()
                {
                    self.runtime.cancel_transfer();
                    self.set_status(StatusKind::Info, "Cancelling transfer…");
                }
            });
            ui.add_space(6.0);
        }
    }

    fn transfer_in_progress(&self) -> bool {
        self.transfer_progress.is_some()
    }

    fn cancel_workflow(&mut self) {
        if self.transfer_in_progress() || self.pending_send_pairing.is_some() {
            self.runtime.cancel_transfer();
            self.set_status(StatusKind::Info, "Cancelling transfer…");
            return;
        }
        if let Some(pending) = self.pending_incoming.as_mut() {
            if let Some(transfer) = pending.transfer.take() {
                self.runtime.decline(transfer);
            }
            self.pending_incoming = None;
            self.set_status(StatusKind::Info, "Incoming transfer declined".to_string());
        }
    }

    fn draw_header(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new("NTRelay")
                .font(inter_semibold(22.0))
                .color(text_primary(ui)),
        );
        ui.label(
            egui::RichText::new(format!("This device: {}", self.identity.name))
                .size(13.0)
                .color(text_primary(ui)),
        );

        let status_color = match self.status_kind {
            StatusKind::Ready => text_primary(ui),
            StatusKind::Info => BTN_BLUE,
            StatusKind::Success => GREEN,
            StatusKind::Error => RED,
        };
        ui.label(egui::RichText::new(&self.status).size(13.0).color(status_color));

        if self.show_open_downloads {
            ui.add_space(6.0);
            if blue_button(
                ui,
                "Open Downloads",
                true,
                true,
                egui::vec2(ui.available_width(), 36.0),
                13.0,
            )
            .clicked()
            {
                open_downloads_in_file_manager();
                self.show_open_downloads = false;
            }
        }
    }

    fn draw_peers(&mut self, ui: &mut egui::Ui) {
        draw_section_toggle(ui, &mut self.show_peers, "Nearby devices");
        if !self.show_peers {
            return;
        }

        ui.add_space(4.0);

        ui.horizontal(|ui| {
            if blue_button(
                ui,
                "↻  Rescan network",
                true,
                false,
                egui::vec2(ui.available_width(), 36.0),
                13.0,
            )
            .clicked()
            {
                self.runtime.rescan_network();
                self.set_status(StatusKind::Info, "Rescanning network…");
            }
        });
        ui.add_space(6.0);

        if self.peers.is_empty() {
            ui.label(
                egui::RichText::new("Searching on your network…")
                    .size(13.0)
                    .color(text_primary(ui)),
            );
            ui.label(
                egui::RichText::new("Both computers must run NTRelay on the same Wi‑Fi.")
                    .size(12.0)
                    .color(text_primary(ui)),
            );
            return;
        }

        egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
            for peer in &self.peers {
                let selected = self.selected_device_id.as_deref() == Some(peer.device_id.as_str());
                let label = peer_row_label(peer, &self.dismissed_new_peers);
                let enabled = true;
                let response = blue_button(
                    ui,
                    &label,
                    enabled,
                    selected,
                    egui::vec2(ui.available_width(), 36.0),
                    14.0,
                );
                if response.clicked() {
                    self.selected_device_id = Some(peer.device_id.clone());
                    self.dismissed_new_peers.insert(peer.device_id.clone());
                }
            }
        });
    }

    fn selected_peer(&self) -> Option<&Peer> {
        let id = self.selected_device_id.as_deref()?;
        self.peers.iter().find(|p| p.device_id == id)
    }

    fn draw_send(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        draw_section_toggle(ui, &mut self.show_send, "Send files");

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
                self.show_send = true;
                self.set_status(StatusKind::Info, "Files dropped — pick a peer and send.");
            }
        }

        if !self.show_send {
            return;
        }

        ui.add_space(4.0);

        if blue_button(ui, "Choose files…", true, false, egui::vec2(ui.available_width(), 38.0), 14.0)
            .clicked()
        {
            if let Some(paths) = rfd::FileDialog::new().pick_files() {
                self.pending_files = paths;
                self.set_status(StatusKind::Info, "Files selected — pick a peer and send.");
            }
        }

        if !self.pending_files.is_empty() {
            ui.add_space(4.0);
            let (count, total) = pending_files_summary(&self.pending_files);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{count} file(s) · {total}"))
                        .size(13.0)
                        .color(text_primary(ui)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Clear all")
                                    .size(12.0)
                                    .color(BTN_BLUE),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        self.pending_files.clear();
                        self.set_status(StatusKind::Info, "File queue cleared.");
                    }
                });
            });

            let mut remove_idx = None;
            egui::ScrollArea::vertical()
                .max_height(100.0)
                .show(ui, |ui| {
                    for (idx, path) in self.pending_files.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let name = path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("file");
                            let size = file_size_label(path);
                            ui.label(
                                egui::RichText::new(format!("{name} ({size})"))
                                    .size(12.0)
                                    .color(text_primary(ui)),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                egui::RichText::new("✕").size(12.0).color(RED),
                                            )
                                            .frame(false),
                                        )
                                        .clicked()
                                    {
                                        remove_idx = Some(idx);
                                    }
                                },
                            );
                        });
                    }
                });
            if let Some(idx) = remove_idx {
                self.pending_files.remove(idx);
                if self.pending_files.is_empty() {
                    self.set_status(StatusKind::Ready, "Ready — choose files to send.");
                }
            }
        }

        ui.add_space(8.0);
        let can_send = self.selected_peer().is_some()
            && !self.pending_files.is_empty()
            && self.pending_send_pairing.is_none();
        let sending = self.transfer_progress.is_some();

        if let Some(pending) = self.pending_send_pairing.clone() {
            ui.add_space(4.0);
            let frame_fill = card_fill(ui.visuals().dark_mode);
            egui::Frame::new()
                .fill(frame_fill)
                .stroke(egui::Stroke::new(1.0_f32, AMBER))
                .inner_margin(10.0)
                .corner_radius(CARD_RADIUS)
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "Pairing with {} — verify this code on both devices:",
                            pending.peer_name
                        ))
                        .size(12.0)
                        .color(text_primary(ui)),
                    );
                    ui.label(
                        egui::RichText::new(&pending.pairing_code)
                            .font(inter_semibold(24.0))
                            .monospace()
                            .color(AMBER),
                    );
                });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if blue_button(ui, "Confirm & send", true, true, egui::vec2(0.0, 38.0), 14.0)
                    .clicked()
                {
                    self.runtime.confirm_send_pairing();
                    self.pending_send_pairing = None;
                    self.transfer_progress = Some(0.08);
                    self.transfer_label = format!("Sending to {}…", pending.peer_name);
                }
                ui.add_space(6.0);
                if blue_button(ui, "Cancel", true, false, egui::vec2(0.0, 38.0), 14.0).clicked()
                {
                    self.runtime.cancel_send_pairing();
                    self.pending_send_pairing = None;
                    self.transfer_progress = None;
                    self.transfer_label.clear();
                }
            });
        }

        if !can_send && !sending && self.pending_send_pairing.is_none() {
            ui.label(
                egui::RichText::new("Select a device and choose files to send.")
                    .size(12.0)
                    .color(text_primary(ui)),
            );
        }

        if green_button(
            ui,
            "Send encrypted",
            can_send && !sending,
            egui::vec2(ui.available_width(), 46.0),
            15.0,
        )
        .clicked()
        && self.pending_send_pairing.is_none()
        {
            if let Some(peer) = self.selected_peer().cloned() {
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
            section_heading(ui, "Incoming connection");
            ui.label(
                egui::RichText::new(format!("From: {}", pending.remote_name))
                    .size(13.0)
                    .color(text_primary(ui)),
            );
            ui.label(
                egui::RichText::new("Accept to receive files.")
                    .size(12.0)
                    .color(text_primary(ui)),
            );

            if pending.needs_pairing {
                ui.add_space(6.0);
                let frame_fill = card_fill(ui.visuals().dark_mode);
                egui::Frame::new()
                    .fill(frame_fill)
                    .stroke(egui::Stroke::new(1.0_f32, AMBER))
                    .inner_margin(10.0)
                    .corner_radius(CARD_RADIUS)
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new("Pairing code — verify on both devices:")
                                .size(12.0)
                                .color(text_primary(ui)),
                        );
                        ui.label(
                            egui::RichText::new(&pending.pairing_code)
                                .font(inter_semibold(24.0))
                                .monospace()
                                .color(AMBER),
                        );
                    });
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if blue_button(ui, "Accept", true, true, egui::vec2(0.0, 38.0), 14.0).clicked() {
                    accept = true;
                }
                ui.add_space(6.0);
                if blue_button(ui, "Decline", true, false, egui::vec2(0.0, 38.0), 14.0).clicked() {
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

    fn draw_log(&mut self, ui: &mut egui::Ui) {
        draw_section_toggle(ui, &mut self.show_activity, "Activity");

        if !self.show_activity {
            return;
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if blue_button(ui, "Clear", !self.logs.is_empty(), false, egui::vec2(72.0, 32.0), 12.0)
                    .clicked()
                {
                    self.logs.clear();
                    if self.status_kind != StatusKind::Error {
                        self.set_status(StatusKind::Ready, "Activity cleared");
                    }
                }
            });
        });
        ui.add_space(4.0);
        egui::ScrollArea::vertical().max_height(100.0).show(ui, |ui| {
            if self.logs.is_empty() {
                ui.label(
                    egui::RichText::new("No activity yet.")
                        .size(12.0)
                        .color(text_primary(ui)),
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
                    text_primary(ui)
                };
                ui.label(egui::RichText::new(line).size(12.0).color(color));
            }
        });
    }
}

impl eframe::App for RelayApp {
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(glass_fill(visuals.dark_mode)).to_array()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let theme = ctx.theme();
        if theme != self.last_theme {
            apply_theme(ctx, theme);
            self.last_theme = theme;
        }

        self.poll_events();
        ctx.request_repaint_after(std::time::Duration::from_millis(200));

        let dark = ctx.style().visuals.dark_mode;
        egui::CentralPanel::default()
            .frame(app_shell_frame(dark))
            .show(ctx, |ui| {
            ui.add_space(8.0);
            self.draw_error_banner(ui);
            self.draw_header(ui);
            self.draw_progress(ui);
            ui.add_space(8.0);
            self.draw_peers(ui);
            ui.add_space(10.0);
            self.draw_send(ui, ctx);
            ui.add_space(10.0);
            self.draw_incoming(ui);
            ui.add_space(10.0);
            self.draw_log(ui);
        });
    }
}

fn peer_row_label(peer: &Peer, dismissed_new: &HashSet<String>) -> String {
    let dot = match peer.presence {
        PeerPresence::Online => "●",
        PeerPresence::Stale => "○",
    };
    let suffix = if peer.is_new && !dismissed_new.contains(&peer.device_id) {
        "  NEW"
    } else if peer.presence == PeerPresence::Stale {
        "  away"
    } else if !peer.reachable {
        "  …"
    } else {
        ""
    };
    format!("{dot} {}{suffix}", peer.name)
}

#[cfg(test)]
mod peer_label_tests {
    use super::*;
    use crate::peer_registry::PeerPresence;
    use std::collections::HashSet;
    use std::net::SocketAddr;

    fn peer(presence: PeerPresence, is_new: bool) -> Peer {
        Peer {
            device_id: "dev-1".into(),
            name: "TestMac".into(),
            addr: SocketAddr::from(([192, 168, 1, 2], 9000)),
            presence,
            is_new,
            reachable: presence == PeerPresence::Online,
        }
    }

    #[test]
    fn label_shows_new_badge() {
        let p = peer(PeerPresence::Online, true);
        assert!(peer_row_label(&p, &HashSet::new()).contains("NEW"));
    }

    #[test]
    fn label_hides_new_after_dismiss() {
        let p = peer(PeerPresence::Online, true);
        let dismissed = HashSet::from(["dev-1".to_string()]);
        assert!(!peer_row_label(&p, &dismissed).contains("NEW"));
    }

    #[test]
    fn label_marks_stale_peer() {
        let p = peer(PeerPresence::Stale, false);
        assert!(peer_row_label(&p, &HashSet::new()).contains("away"));
    }
}

fn draw_section_toggle(ui: &mut egui::Ui, open: &mut bool, title: &str) {
    ui.horizontal(|ui| {
        ui.checkbox(open, "");
        ui.label(
            egui::RichText::new(title)
                .font(inter_semibold(15.0))
                .color(text_primary(ui)),
        );
    });
}

fn section_heading(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .font(inter_semibold(15.0))
            .color(text_primary(ui)),
    );
}

fn text_primary(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::WHITE
    } else {
        egui::Color32::BLACK
    }
}

fn inter_semibold(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(INTER_SEMIBOLD.into()))
}

fn pending_files_summary(paths: &[PathBuf]) -> (usize, String) {
    let count = paths.len();
    let total_bytes: u64 = paths
        .iter()
        .filter_map(|p| fs::metadata(p).ok().map(|m| m.len()))
        .sum();
    (count, format_bytes(total_bytes))
}

fn file_size_label(path: &Path) -> String {
    fs::metadata(path)
        .map(|m| format_bytes(m.len()))
        .unwrap_or_else(|_| "unknown".to_string())
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

fn open_downloads_in_file_manager() {
    if let Ok(path) = download_dir() {
        open_path_in_file_manager(&path);
    }
}

#[cfg(target_os = "macos")]
fn open_path_in_file_manager(path: &Path) {
    let _ = std::process::Command::new("open").arg(path).spawn();
}

#[cfg(target_os = "linux")]
fn open_path_in_file_manager(path: &Path) {
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

#[cfg(target_os = "windows")]
fn open_path_in_file_manager(path: &Path) {
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn open_path_in_file_manager(_path: &Path) {}

fn configure_inter_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert(
        "inter-regular".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/fonts/Inter-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "inter-semibold".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/fonts/Inter-SemiBold.ttf"
        ))),
    );

    fonts
        .families
        .entry(egui::FontFamily::Name(INTER_SEMIBOLD.into()))
        .or_default()
        .insert(0, "inter-semibold".to_owned());

    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "inter-regular".to_owned());

    ctx.set_fonts(fonts);
}

fn blue_button(
    ui: &mut egui::Ui,
    label: &str,
    enabled: bool,
    emphasized: bool,
    min_size: egui::Vec2,
    font_size: f32,
) -> egui::Response {
    pill_button(
        ui,
        label,
        enabled,
        ButtonKind::Blue,
        emphasized,
        min_size,
        font_size,
    )
}

fn green_button(
    ui: &mut egui::Ui,
    label: &str,
    enabled: bool,
    min_size: egui::Vec2,
    font_size: f32,
) -> egui::Response {
    pill_button(
        ui,
        label,
        enabled,
        ButtonKind::Green,
        true,
        min_size,
        font_size,
    )
}

#[derive(Clone, Copy)]
enum ButtonKind {
    Blue,
    Green,
}

fn pill_button(
    ui: &mut egui::Ui,
    label: &str,
    enabled: bool,
    kind: ButtonKind,
    emphasized: bool,
    min_size: egui::Vec2,
    font_size: f32,
) -> egui::Response {
    let width = if min_size.x > 0.0 {
        min_size.x
    } else {
        ui.available_width().max(100.0) / 2.0 - 4.0
    };
    let height = min_size.y.max(44.0);
    let size = egui::vec2(width, height);
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_at_least(size, sense);

    if ui.is_rect_visible(rect) {
        let hovered = enabled && response.hovered();
        let pressed = enabled && response.is_pointer_button_down_on();
        let (fill, stroke, stroke_w) = button_style(kind, enabled, emphasized, hovered, pressed);

        if enabled {
            let shadow = rect.translate(egui::vec2(0.0, 4.0));
            ui.painter().rect_filled(
                shadow,
                BTN_RADIUS + 2.0,
                egui::Color32::from_black_alpha(if pressed { 25 } else if hovered { 55 } else { 40 }),
            );
        }

        ui.painter()
            .rect_filled(rect, BTN_RADIUS, fill);
        ui.painter().rect_stroke(
            rect,
            BTN_RADIUS,
            egui::Stroke::new(stroke_w, stroke),
            egui::StrokeKind::Inside,
        );

        if enabled && !pressed {
            let shine = egui::Rect::from_min_max(
                rect.min + egui::vec2(3.0, 2.0),
                rect.min + egui::vec2(rect.width() - 3.0, rect.height() * 0.45),
            );
            ui.painter().rect_filled(
                shine,
                BTN_RADIUS,
                egui::Color32::from_white_alpha(if hovered { 36 } else { 22 }),
            );
        }

        if emphasized && enabled {
            ui.painter().rect_stroke(
                rect.expand(1.5),
                BTN_RADIUS + 1.5,
                egui::Stroke::new(
                    1.5,
                    egui::Color32::from_white_alpha(if hovered { 90 } else { 55 }),
                ),
                egui::StrokeKind::Outside,
            );
        }

        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            inter_semibold(font_size),
            egui::Color32::WHITE,
        );
    }

    response
}

fn button_style(
    kind: ButtonKind,
    enabled: bool,
    emphasized: bool,
    hovered: bool,
    pressed: bool,
) -> (egui::Color32, egui::Color32, f32) {
    if !enabled {
        return match kind {
            ButtonKind::Blue => (BTN_BLUE_DISABLED, BTN_STROKE, 1.5),
            ButtonKind::Green => (BTN_GREEN_DISABLED, BTN_GREEN_STROKE, 1.5),
        };
    }

    if pressed {
        return match kind {
            ButtonKind::Blue => (BTN_BLUE_DARK, BTN_STROKE, 2.5),
            ButtonKind::Green => (
                egui::Color32::from_rgb(22, 163, 74),
                BTN_GREEN_STROKE,
                2.5,
            ),
        };
    }

    match kind {
        ButtonKind::Blue => {
            let fill = if hovered {
                BTN_BLUE_HOVER
            } else if emphasized {
                BTN_BLUE_DARK
            } else {
                BTN_BLUE
            };
            (fill, BTN_STROKE, if hovered || emphasized { 2.5 } else { 2.0 })
        }
        ButtonKind::Green => {
            let fill = if hovered { BTN_GREEN_HOVER } else { BTN_GREEN };
            (fill, BTN_GREEN_STROKE, if hovered { 2.5 } else { 2.0 })
        }
    }
}

fn glass_fill(dark: bool) -> egui::Color32 {
    if dark {
        egui::Color32::from_rgba_unmultiplied(24, 26, 32, 238)
    } else {
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 232)
    }
}

fn card_fill(dark: bool) -> egui::Color32 {
    if dark {
        egui::Color32::from_rgba_unmultiplied(38, 42, 52, 210)
    } else {
        egui::Color32::from_rgba_unmultiplied(255, 251, 235, 220)
    }
}

fn app_shell_frame(dark: bool) -> egui::Frame {
    egui::Frame::new()
        .fill(glass_fill(dark))
        .inner_margin(egui::Margin::symmetric(18, 16))
}

fn apply_theme(ctx: &egui::Context, theme: egui::Theme) {
    let mut visuals = match theme {
        egui::Theme::Dark => egui::Visuals::dark(),
        egui::Theme::Light => egui::Visuals::light(),
    };

    let text = if visuals.dark_mode {
        egui::Color32::WHITE
    } else {
        egui::Color32::BLACK
    };
    visuals.override_text_color = Some(text);

    if visuals.dark_mode {
        visuals.panel_fill = egui::Color32::TRANSPARENT;
        visuals.window_fill = egui::Color32::TRANSPARENT;
        visuals.extreme_bg_color = glass_fill(true);
    } else {
        visuals.panel_fill = egui::Color32::TRANSPARENT;
        visuals.window_fill = egui::Color32::TRANSPARENT;
        visuals.extreme_bg_color = glass_fill(false);
    }

    visuals.window_corner_radius = egui::CornerRadius::ZERO;
    visuals.window_stroke = egui::Stroke::NONE;

    visuals.selection.bg_fill = BTN_BLUE;
    visuals.selection.stroke.color = BTN_STROKE;
    visuals.widgets.noninteractive.fg_stroke.color = text;
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::new(14.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::new(14.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Heading,
        inter_semibold(18.0),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        egui::FontId::new(18.0, egui::FontFamily::Monospace),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        egui::FontId::new(12.0, egui::FontFamily::Proportional),
    );
    style.spacing.button_padding = egui::vec2(16.0, 10.0);
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.indent = 14.0;
    style.visuals.widgets.inactive.corner_radius = BTN_RADIUS.into();
    style.visuals.widgets.hovered.corner_radius = BTN_RADIUS.into();
    style.visuals.widgets.active.corner_radius = BTN_RADIUS.into();
    ctx.set_style(style);
}

pub fn run(identity: Identity, trust: TrustStore) -> eframe::Result<()> {
    let runtime = crate::runtime::spawn(identity.clone(), trust)
        .map_err(|e| eframe::Error::AppCreation(format!("{e}").into()))?;

    const W: f32 = 380.0;
    const H: f32 = 680.0;

    let icon = std::sync::Arc::new(
        eframe::icon_data::from_png_bytes(include_bytes!("../assets/icons/icon-ntr-blue.png"))
            .expect("load app icon"),
    );

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([W, H])
            .with_min_inner_size([W, H])
            .with_max_inner_size([W, H])
            .with_resizable(false)
            .with_title("NTRelay")
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "NTRelay",
        options,
        Box::new(move |cc| Ok(Box::new(RelayApp::new(cc, identity, runtime)))),
    )
}
