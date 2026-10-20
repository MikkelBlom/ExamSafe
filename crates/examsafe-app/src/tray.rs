//! Native tray icon (no WebView): hover shows a status tooltip, left-click opens the window,
//! right-click shows the menu (Open, Make safe, Restore, Quit).

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

const ICON_SIZE: u32 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayStatus {
    Neutral,
    Busy,
    Safe,
    ExamMode,
    Problem,
}

impl TrayStatus {
    fn rgb(self) -> [u8; 3] {
        match self {
            Self::Neutral => [0x8e, 0x8e, 0x93],
            Self::Busy => [0x0a, 0x84, 0xff],
            Self::Safe => [0x28, 0xa7, 0x45],
            Self::ExamMode => [0x5e, 0x5c, 0xe6],
            Self::Problem => [0xe5, 0x37, 0x2c],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Open,
    MakeSafe,
    Restore,
    Quit,
}

/// Which menu actions make sense in the current phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrayMenuState {
    pub make_safe: bool,
    pub restore: bool,
}

pub struct Tray {
    icon: TrayIcon,
    make_safe: MenuItem,
    restore: MenuItem,
    commands: Vec<(MenuId, TrayCommand)>,
}

impl Tray {
    pub fn create() -> Result<Self, Box<dyn std::error::Error>> {
        let open = MenuItem::new("Open ExamSafe", true, None);
        let make_safe = MenuItem::new("Make my PC exam-safe", false, None);
        let restore = MenuItem::new("Restore my PC", false, None);
        let quit = MenuItem::new("Quit ExamSafe", true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &open,
            &PredefinedMenuItem::separator(),
            &make_safe,
            &restore,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("ExamSafe")
            .with_icon(make_icon(TrayStatus::Neutral)?)
            .build()?;
        let commands = vec![
            (open.id().clone(), TrayCommand::Open),
            (make_safe.id().clone(), TrayCommand::MakeSafe),
            (restore.id().clone(), TrayCommand::Restore),
            (quit.id().clone(), TrayCommand::Quit),
        ];
        Ok(Self {
            icon,
            make_safe,
            restore,
            commands,
        })
    }

    /// Routes tray clicks and menu picks to `on_command`. Handlers run on the UI thread's
    /// message loop; `on_command` must hand work to the Slint event loop.
    pub fn install_handlers(&self, on_command: fn(TrayCommand)) {
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                on_command(TrayCommand::Open);
            }
        }));
        let commands = self.commands.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some((_, command)) = commands.iter().find(|(id, _)| *id == event.id) {
                on_command(*command);
            }
        }));
    }

    /// Greys out actions that do nothing in the current phase.
    pub fn set_menu_state(&self, state: TrayMenuState) {
        self.make_safe.set_enabled(state.make_safe);
        self.restore.set_enabled(state.restore);
    }

    pub fn set_status(&self, status: TrayStatus, tooltip: &str) {
        if let Err(error) = make_icon(status).and_then(|icon| Ok(self.icon.set_icon(Some(icon))?)) {
            eprintln!("examsafe: could not update tray icon: {error}");
        }
        if let Err(error) = self.icon.set_tooltip(Some(tooltip)) {
            eprintln!("examsafe: could not update tray tooltip: {error}");
        }
    }
}

fn make_icon(status: TrayStatus) -> Result<Icon, Box<dyn std::error::Error>> {
    Ok(Icon::from_rgba(
        render_icon(ICON_SIZE, status.rgb()),
        ICON_SIZE,
        ICON_SIZE,
    )?)
}

/// Anti-aliased disc in `rgb` with a white centre dot, as RGBA bytes.
pub fn render_icon(size: u32, rgb: [u8; 3]) -> Vec<u8> {
    let center = f64::from(size) / 2.0;
    let outer = center - 1.0;
    let inner = f64::from(size) * 0.17;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let dx = f64::from(x) + 0.5 - center;
            let dy = f64::from(y) + 0.5 - center;
            let distance = (dx * dx + dy * dy).sqrt();
            let disc = (outer - distance + 0.5).clamp(0.0, 1.0);
            let dot = (inner - distance + 0.5).clamp(0.0, 1.0);
            let mix = |channel: u8| {
                let value = f64::from(channel) * (1.0 - dot) + 255.0 * dot;
                value.round().clamp(0.0, 255.0) as u8
            };
            let alpha = (disc * 255.0).round().clamp(0.0, 255.0) as u8;
            pixels.extend_from_slice(&[mix(rgb[0]), mix(rgb[1]), mix(rgb[2]), alpha]);
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(pixels: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    }

    #[test]
    fn has_one_rgba_pixel_per_position() {
        assert_eq!(render_icon(32, [1, 2, 3]).len(), 32 * 32 * 4);
    }

    #[test]
    fn corners_are_transparent_and_centre_is_white() {
        let pixels = render_icon(32, [10, 20, 30]);
        assert_eq!(pixel(&pixels, 32, 0, 0)[3], 0);
        assert_eq!(pixel(&pixels, 32, 16, 16), [255, 255, 255, 255]);
    }

    #[test]
    fn ring_uses_the_status_colour() {
        let pixels = render_icon(32, [10, 20, 30]);
        assert_eq!(pixel(&pixels, 32, 16, 3), [10, 20, 30, 255]);
    }
}
