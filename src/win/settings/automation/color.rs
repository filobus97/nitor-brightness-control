//! The color temperature card of the Automation section (Plus): switch, day and night
//! temperature, the experimental warm limit, the Night Light hint and each display's method.

use super::*;
use crate::auto::color::{MAX_K, Method, UI_MIN_K, WARM_LIMIT_K};
use crate::config::ColorMethodPref;

/// Temperature slider granularity (kelvin), the planner's own step.
const STEP_K: u32 = 100;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::win::settings) enum ColorCtl {
    Enabled,
    Day,
    Night,
    /// "Allow warmer than 4000 K (experimental)".
    Warm,
    /// Enable the extended gamma range (administrator).
    WarmRange,
    /// Open the Windows Night Light settings.
    NightLight,
    /// Monitor `i`'s method choice.
    Method(usize, ColorMethodPref),
    /// "Restore monitor colors" for monitor `i`.
    Restore(usize),
}

fn ctl(c: ColorCtl) -> Ctl {
    Ctl::Auto(AutoCtl::Color(c))
}

impl State {
    pub(super) fn color_layout(&self, b: &mut Builder) {
        use PlusKey as K;
        let a = &self.snap.config.auto;
        b.heading(Key::Plus(K::SecColor));
        b.begin_card();
        b.switch_row(
            ctl(ColorCtl::Enabled),
            self.pt(K::ColorEnabled),
            Some(self.pt(K::ColorEnabledHint)),
        );
        if a.color_enabled {
            self.kelvin_row(b, ColorCtl::Day, K::ColorDay, a.day_kelvin);
            self.kelvin_row(b, ColorCtl::Night, K::ColorNight, a.night_kelvin);
            b.switch_row(ctl(ColorCtl::Warm), self.pt(K::ColorWarm), None);
            if a.experimental_warm && !self.snap.warm_range_ok {
                self.note_row(b, self.pt(K::ColorWarmNote));
                self.color_button_row(b, ColorCtl::WarmRange);
            }
            if self.snap.night_light {
                self.note_row(b, self.pt(K::ColorNightLightNote));
                self.color_button_row(b, ColorCtl::NightLight);
            }
        }
        for (i, m) in self.snap.monitors.iter().enumerate() {
            if m.internal {
                // A built-in panel has no hardware choice: software only.
                let row = b.row(ROW_H);
                let value = Rect::new(row.l + b.d(220.0), row.t, b.edge(row), row.b);
                b.nodes.push(Node::Value(self.method_caption(i), value));
                b.label(row, b.left(row), value.l, self.mon_name(i), None);
                continue;
            }
            let pref = self.method_pref(i);
            let row = b.row(ROW_H);
            let prefs = [
                ColorMethodPref::Auto,
                ColorMethodPref::Ddc,
                ColorMethodPref::Gamma,
            ];
            let widths =
                prefs.map(|p| self.method_seg_label(p).chars().count() as f32 * CHAR_W + 24.0);
            let g = b.slot(row, b.edge(row), widths.iter().sum(), SEG_H);
            b.nodes.push(Node::Group(g));
            let mut left = g.l;
            for (p, w) in prefs.into_iter().zip(widths) {
                let r = Rect::new(left, g.t, left + b.d(w), g.b);
                b.ctl(ctl(ColorCtl::Method(i, p)), r);
                left = r.r;
            }
            b.label(row, b.left(row), g.l, self.mon_name(i), None);
            self.note_row(b, self.method_caption(i));
            if pref == ColorMethodPref::Ddc {
                self.note_row(b, self.pt(K::ColorHwNote));
                self.note_row(b, self.pt(K::ColorRestoreNote));
                self.color_button_row(b, ColorCtl::Restore(i));
            }
        }
        b.end_card(SECTION_GAP);
    }

    fn method_pref(&self, i: usize) -> ColorMethodPref {
        self.snap
            .config
            .monitors
            .get(&self.snap.monitors[i].id)
            .map(|s| s.color_method)
            .unwrap_or_default()
    }

    fn method_seg_label(&self, p: ColorMethodPref) -> String {
        self.pt(match p {
            ColorMethodPref::Auto => PlusKey::ColorSegAuto,
            ColorMethodPref::Ddc => PlusKey::ColorSegHardware,
            ColorMethodPref::Gamma => PlusKey::ColorSegSoftware,
        })
    }

    /// What monitor `i` actually uses (or why it cannot).
    fn method_caption(&self, i: usize) -> String {
        let m = &self.snap.monitors[i];
        let ddc = self.method_pref(i) == ColorMethodPref::Ddc && !m.internal;
        self.pt(match m.color_method {
            _ if m.hdr => PlusKey::ColorMethodHdr,
            _ if ddc && m.color_unsupported => PlusKey::ColorMethodUnsupported,
            Some(method) => method_key(method),
            None if ddc => PlusKey::ColorMethodHw,
            None => PlusKey::ColorMethodGamma,
        })
    }

    /// Title, slider and value of a day/night temperature.
    fn kelvin_row(&self, b: &mut Builder, c: ColorCtl, title: PlusKey, kelvin: u32) {
        let row = b.row(ROW_H);
        let value = Rect::new(b.edge(row) - b.d(64.0), row.t, b.edge(row), row.b);
        let slider = b.slot(row, value.l - b.d(8.0), 200.0, SLIDER_H);
        b.nodes.push(Node::Value(format!("{kelvin} K"), value));
        b.ctl(ctl(c), slider);
        b.label(row, b.left(row), slider.l, self.pt(title), None);
    }

    /// A right-aligned button without a title (the note above explains it).
    fn color_button_row(&self, b: &mut Builder, c: ColorCtl) {
        let row = b.row(ROW_H);
        let r = b.slot(row, b.edge(row), self.color_btn_width(c), BTN);
        b.ctl(ctl(c), r);
    }

    fn color_btn_label(&self, c: ColorCtl) -> String {
        self.pt(match c {
            ColorCtl::WarmRange => PlusKey::ColorWarmBtn,
            ColorCtl::Restore(_) => PlusKey::ColorRestoreBtn,
            _ => PlusKey::ColorNightLightBtn,
        })
    }

    fn color_btn_width(&self, c: ColorCtl) -> f32 {
        self.color_btn_label(c).chars().count() as f32 * (CHAR_W + 0.5) + 28.0
    }

    /// The temperature of slider `c`, and the warmest the user may pick now.
    fn kelvin_of(&self, c: ColorCtl) -> u32 {
        let a = &self.snap.config.auto;
        if c == ColorCtl::Day {
            a.day_kelvin
        } else {
            a.night_kelvin
        }
    }

    pub(super) fn color_press(&mut self, layout: &Layout, c: ColorCtl, x: f32) -> Option<Action> {
        let a = &self.snap.config.auto;
        let edit = match c {
            ColorCtl::Enabled => AutoEdit::ColorEnabled(!a.color_enabled),
            ColorCtl::Warm => AutoEdit::Warm(!a.experimental_warm),
            ColorCtl::WarmRange => AutoEdit::WarmRange,
            ColorCtl::Day | ColorCtl::Night => {
                self.drag = Some(Drag::Level(AutoCtl::Color(c)));
                return self.color_drag(layout, c, x);
            }
            ColorCtl::NightLight => {
                util::open_url("ms-settings:nightlight");
                return None;
            }
            ColorCtl::Method(i, pref) => {
                let id = self.snap.monitors.get(i)?.id.clone();
                return (self.method_pref(i) != pref)
                    .then_some(Action::Monitor(id, MonitorEdit::ColorMethod(pref)));
            }
            ColorCtl::Restore(i) => {
                let id = self.snap.monitors.get(i)?.id.clone();
                return Some(Action::Monitor(id, MonitorEdit::RestoreColors));
            }
        };
        Some(Action::Auto(edit))
    }

    /// Move a temperature slider to the pointer (steps of 100 K, never warmer than allowed).
    pub(super) fn color_drag(&self, layout: &Layout, c: ColorCtl, x: f32) -> Option<Action> {
        let r = layout.rect_of(ctl(c))?;
        let raw = UI_MIN_K as f32 + frac_at(r, self.scale(), x) * (MAX_K - UI_MIN_K) as f32;
        let floor = if self.snap.config.auto.experimental_warm {
            UI_MIN_K
        } else {
            WARM_LIMIT_K
        };
        let k = ((raw / STEP_K as f32).round() as u32 * STEP_K).clamp(floor, MAX_K);
        (k != self.kelvin_of(c)).then_some(Action::Auto(if c == ColorCtl::Day {
            AutoEdit::DayKelvin(k)
        } else {
            AutoEdit::NightKelvin(k)
        }))
    }

    pub(super) fn draw_color(&self, cv: &Canvas, c: ColorCtl, r: Rect, hot: bool) {
        let a = &self.snap.config.auto;
        match c {
            ColorCtl::Enabled => cv.switch(r, a.color_enabled, self.theme.accent),
            ColorCtl::Warm => cv.switch(r, a.experimental_warm, self.theme.accent),
            ColorCtl::Day | ColorCtl::Night => {
                let frac =
                    (self.kelvin_of(c).saturating_sub(UI_MIN_K)) as f32 / (MAX_K - UI_MIN_K) as f32;
                let pressed = self.drag == Some(Drag::Level(AutoCtl::Color(c)));
                cv.slider(r, (0.0, frac), false, self.theme.accent, (hot, pressed));
            }
            ColorCtl::WarmRange | ColorCtl::NightLight | ColorCtl::Restore(_) => {
                self.draw_button(cv, r, &self.color_btn_label(c), hot)
            }
            ColorCtl::Method(i, pref) => self.draw_segment(
                cv,
                r,
                &self.method_seg_label(pref),
                self.method_pref(i) == pref,
                hot,
            ),
        }
    }
}

fn method_key(m: Method) -> PlusKey {
    match m {
        Method::Hardware => PlusKey::ColorMethodHw,
        Method::Gamma => PlusKey::ColorMethodGamma,
        Method::Hdr => PlusKey::ColorMethodHdr,
    }
}
