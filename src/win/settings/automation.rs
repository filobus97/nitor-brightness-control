//! Nitor Plus part of the Settings window: the Automation and Location sections and the
//! place search. A child module of `settings`, so it builds on that file's private pieces.

use super::*;
use crate::auto::geocode::{self, Place};
use crate::auto::map::round2;
use crate::win::geo;

const PLACE_H: f32 = 40.0;
const FIELD_W: f32 = 120.0;
const SEARCH_FIELD_W: f32 = 170.0;
/// Rough DIP widths of 14 pt text, to size segments and buttons from their labels.
const CHAR_W: f32 = 7.0;
/// Characters of a 12 pt note that fit on one line of a card (conservative).
const NOTE_LINE_CHARS: f32 = 66.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum AutoCtl {
    Enabled,
    UseSensor,
    Weather,
    Day,
    Night,
    /// Option `k` of "after a manual change".
    Override(usize),
    /// Option `k` of the location source.
    Source(usize),
    Search,
    Map,
    Adaptive,
    /// Search result `k`.
    Place(usize),
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Status {
    #[default]
    Idle,
    Searching,
    NoResults {
        /// The query was only digits: suggest the city name.
        digits: bool,
    },
    Failed,
}

/// State of the place search.
#[derive(Default)]
pub(super) struct AutoUi {
    /// What the search field holds (committed when it loses focus).
    query: String,
    places: Vec<Place>,
    status: Status,
    /// Number of the latest search; older answers are ignored.
    ticket: u64,
}

impl State {
    fn pt(&self, key: PlusKey) -> String {
        tr(self.lang(), Key::Plus(key)).to_string()
    }

    // --- layout -----------------------------------------------------------

    pub(super) fn automation_layout(&self, b: &mut Builder) {
        use PlusKey as K;
        let a = &self.snap.config.auto;

        b.heading(Key::Plus(K::SecAutomation));
        b.begin_card();
        b.switch_row(
            Ctl::Auto(AutoCtl::Enabled),
            self.pt(K::AutoEnabled),
            Some(self.pt(K::AutoEnabledHint)),
        );
        self.level_row(b, AutoCtl::Day, K::AutoDay, a.day_brightness);
        self.level_row(b, AutoCtl::Night, K::AutoNight, a.night_brightness);
        self.note_row(b, self.pt(K::AutoLevelsNote));
        if self.snap.sensor_available {
            b.switch_row(Ctl::Auto(AutoCtl::UseSensor), self.pt(K::AutoSensor), None);
        }
        if self.snap.adaptive_conflict {
            self.note_row(b, self.pt(K::AutoAdaptiveNote));
            self.button_row(b, AutoCtl::Adaptive, self.pt(K::AutoAdaptiveTitle));
        }
        b.switch_row(
            Ctl::Auto(AutoCtl::Weather),
            self.pt(K::AutoWeather),
            Some(self.pt(K::AutoWeatherHint)),
        );
        if let Some(text) = self.weather_status_text() {
            self.note_row(b, text);
        }
        self.seg_row(
            b,
            self.pt(K::AutoOverride),
            [AutoCtl::Override(0), AutoCtl::Override(1)],
        );
        b.end_card(SECTION_GAP);

        b.heading(Key::Plus(K::SecLocation));
        b.begin_card();
        let row = b.row(ROW_H);
        let text = match (a.latitude, a.longitude) {
            (Some(lat), Some(lon)) => format!("{lat:.2}, {lon:.2}"),
            _ => self.pt(K::LocNotSet),
        };
        let value = Rect::new(row.l + b.d(220.0), row.t, b.edge(row), row.b);
        b.nodes.push(Node::Value(text, value));
        b.label(row, b.left(row), value.l, self.pt(K::LocCurrent), None);
        self.seg_row(
            b,
            self.pt(K::LocSource),
            [AutoCtl::Source(0), AutoCtl::Source(1)],
        );
        if a.location_source == LocationSource::Windows {
            self.note_row(b, self.pt(K::LocWindowsNote));
        } else {
            self.field_row(b, Kind::Lat, self.pt(K::LocLat));
            self.field_row(b, Kind::Lon, self.pt(K::LocLon));
            self.search_rows(b);
            self.button_row(b, AutoCtl::Map, self.pt(K::LocMap));
        }
        b.end_card(SECTION_GAP);
    }

    /// Weather needs a location to ask about; everything else is always available.
    pub(super) fn auto_enabled(&self, c: AutoCtl) -> bool {
        let a = &self.snap.config.auto;
        c != AutoCtl::Weather || (a.latitude.is_some() && a.longitude.is_some())
    }

    /// "Last update: HH:MM, cloud cover N%" or the last error.
    fn weather_status_text(&self) -> Option<String> {
        use crate::auto::weather::Status;
        if !self.snap.config.auto.weather {
            return None;
        }
        match &self.snap.weather_status {
            Status::None => None,
            Status::Ok { at: (h, m), cover } => Some(
                self.pt(PlusKey::AutoWeatherOk)
                    .replacen("{}", &format!("{h:02}:{m:02}"), 1)
                    .replacen("{}", &cover.to_string(), 1),
            ),
            Status::Failed(e) => Some(tr_with(
                self.lang(),
                Key::Plus(PlusKey::AutoWeatherFailed),
                e,
            )),
        }
    }

    /// Title, slider and value of a day/night level.
    fn level_row(&self, b: &mut Builder, c: AutoCtl, title: PlusKey, level: u8) {
        let row = b.row(ROW_H);
        let value = Rect::new(b.edge(row) - b.d(48.0), row.t, b.edge(row), row.b);
        let slider = b.slot(row, value.l - b.d(8.0), 200.0, SLIDER_H);
        b.nodes.push(Node::Value(format!("{level}%"), value));
        b.ctl(Ctl::Auto(c), slider);
        b.label(row, b.left(row), slider.l, self.pt(title), None);
    }

    /// A caption spanning the card, as tall as its text needs.
    fn note_row(&self, b: &mut Builder, text: String) {
        let lines = (text.chars().count() as f32 / NOTE_LINE_CHARS)
            .ceil()
            .max(1.0);
        let row = b.row(12.0 + 16.0 * lines);
        let rect = Rect::new(b.left(row), row.t, b.edge(row), row.b);
        b.nodes.push(Node::Note(text, rect));
    }

    /// Segmented choice sized from its option labels.
    fn seg_row<const N: usize>(&self, b: &mut Builder, title: String, segs: [AutoCtl; N]) {
        let row = b.row(ROW_H);
        let widths = segs.map(|c| self.seg_label(c).chars().count() as f32 * CHAR_W + 24.0);
        let g = b.slot(row, b.edge(row), widths.iter().sum(), SEG_H);
        b.nodes.push(Node::Group(g));
        let mut left = g.l;
        for (c, w) in segs.into_iter().zip(widths) {
            let r = Rect::new(left, g.t, left + b.d(w), g.b);
            b.ctl(Ctl::Auto(c), r);
            left = r.r;
        }
        b.label(row, b.left(row), g.l, title, None);
    }

    fn button_row(&self, b: &mut Builder, c: AutoCtl, title: String) {
        let row = b.row(ROW_H);
        let r = b.slot(row, b.edge(row), self.btn_width(c), BTN);
        b.ctl(Ctl::Auto(c), r);
        b.label(row, b.left(row), r.l, title, None);
    }

    fn field_row(&self, b: &mut Builder, kind: Kind, title: String) {
        let row = b.row(ROW_H);
        let rect = b.slot(row, b.edge(row), FIELD_W, FIELD_H);
        b.nodes.push(Node::Field { rect, mon: 0, kind });
        b.label(row, b.left(row), rect.l, title, None);
    }

    /// The search field and button, then the result rows or a status line.
    fn search_rows(&self, b: &mut Builder) {
        use PlusKey as K;
        let row = b.row(ROW_H);
        let btn = b.slot(row, b.edge(row), self.btn_width(AutoCtl::Search), BTN);
        b.ctl(Ctl::Auto(AutoCtl::Search), btn);
        let rect = b.slot(row, btn.l - b.d(8.0), SEARCH_FIELD_W, FIELD_H);
        b.nodes.push(Node::Field {
            rect,
            mon: 0,
            kind: Kind::Search,
        });
        b.label(row, b.left(row), rect.l, self.pt(K::LocSearch), None);

        if !self.auto.places.is_empty() {
            for k in 0..self.auto.places.len() {
                let row = b.row(PLACE_H);
                b.ctl(Ctl::Auto(AutoCtl::Place(k)), row);
            }
            return;
        }
        let note = match self.auto.status {
            Status::Idle => K::LocSearchNote,
            Status::Searching => K::LocSearching,
            Status::NoResults { digits: false } => K::LocNoResults,
            Status::NoResults { digits: true } => K::LocNoResultsDigits,
            Status::Failed => K::LocSearchFailed,
        };
        self.note_row(b, self.pt(note));
    }

    fn btn_width(&self, c: AutoCtl) -> f32 {
        self.btn_label(c).chars().count() as f32 * (CHAR_W + 0.5) + 28.0
    }

    fn btn_label(&self, c: AutoCtl) -> String {
        self.pt(match c {
            AutoCtl::Search => PlusKey::LocSearchBtn,
            AutoCtl::Map => PlusKey::LocMapBtn,
            _ => PlusKey::AutoAdaptiveBtn,
        })
    }

    fn seg_label(&self, c: AutoCtl) -> String {
        let a = &self.snap.config.auto;
        match c {
            AutoCtl::Override(0) => self.pt(PlusKey::AutoOverrideTransition),
            AutoCtl::Override(_) if a.override_minutes == 60 => self.pt(PlusKey::AutoOverrideHour),
            AutoCtl::Override(_) => tr_with(
                self.lang(),
                Key::Plus(PlusKey::AutoOverrideMinutes),
                &a.override_minutes.to_string(),
            ),
            AutoCtl::Source(0) => self.pt(PlusKey::LocManual),
            _ => self.pt(PlusKey::LocWindows),
        }
    }

    fn seg_selected(&self, c: AutoCtl) -> bool {
        let a = &self.snap.config.auto;
        match c {
            AutoCtl::Override(k) => (a.override_mode == OverrideMode::Minutes) == (k == 1),
            AutoCtl::Source(k) => (a.location_source == LocationSource::Windows) == (k == 1),
            _ => false,
        }
    }

    // --- input ------------------------------------------------------------

    pub(super) fn auto_press(&mut self, layout: &Layout, c: AutoCtl, x: f32) -> Option<Action> {
        let a = &self.snap.config.auto;
        let edit = match c {
            AutoCtl::Enabled => AutoEdit::Enabled(!a.enabled),
            AutoCtl::UseSensor => AutoEdit::UseSensor(!a.use_sensor),
            AutoCtl::Weather => AutoEdit::Weather(!a.weather),
            AutoCtl::Day | AutoCtl::Night => {
                self.drag = Some(Drag::Level(c));
                return self.auto_drag(layout, c, x);
            }
            AutoCtl::Override(k) => AutoEdit::Override(if k == 0 {
                OverrideMode::UntilTransition
            } else {
                OverrideMode::Minutes
            }),
            AutoCtl::Source(k) => AutoEdit::Source(if k == 0 {
                LocationSource::Manual
            } else {
                LocationSource::Windows
            }),
            AutoCtl::Search => {
                self.start_search();
                return None;
            }
            AutoCtl::Map => {
                mappick::open(
                    self.hwnd,
                    self.lang(),
                    self.dpi,
                    a.latitude.zip(a.longitude),
                    |lat, lon| emit(Some(Action::Auto(AutoEdit::Place(lat, lon)))),
                );
                return None;
            }
            AutoCtl::Adaptive => {
                util::open_url("ms-settings:display");
                return None;
            }
            AutoCtl::Place(k) => {
                let p = self.auto.places.get(k)?;
                let edit = AutoEdit::Place(round2(p.latitude), round2(p.longitude));
                self.auto.places.clear();
                self.auto.status = Status::Idle;
                self.sync_all();
                edit
            }
        };
        Some(Action::Auto(edit))
    }

    /// Move a level slider to the pointer.
    pub(super) fn auto_drag(&self, layout: &Layout, c: AutoCtl, x: f32) -> Option<Action> {
        let r = layout.rect_of(Ctl::Auto(c))?;
        let v = (frac_at(r, self.scale(), x) * 100.0).round() as u8;
        let a = &self.snap.config.auto;
        let edit = match c {
            AutoCtl::Day => (v != a.day_brightness).then_some(AutoEdit::Day(v)),
            AutoCtl::Night => (v != a.night_brightness).then_some(AutoEdit::Night(v)),
            _ => None,
        };
        edit.map(Action::Auto)
    }

    /// Search click: one request, in the background.
    fn start_search(&mut self) {
        let query = self.auto.query.trim().to_string();
        if query.is_empty() {
            return;
        }
        self.auto.ticket += 1;
        self.auto.places.clear();
        self.auto.status = Status::Searching;
        geo::spawn(
            query,
            self.lang(),
            self.hwnd,
            WM_SEARCH_DONE,
            self.auto.ticket,
        );
        self.sync_all();
    }

    /// `WM_SEARCH_DONE`: show the answer of the latest search.
    pub(super) fn search_done(&mut self) {
        match geo::take(self.auto.ticket) {
            None => return,
            Some(None) => self.auto.status = Status::Failed,
            Some(Some(places)) if places.is_empty() => {
                let digits = geocode::is_numeric_query(&self.auto.query);
                self.auto.status = Status::NoResults { digits };
            }
            Some(Some(places)) => {
                self.auto.places = places;
                self.auto.status = Status::Idle;
            }
        }
        self.sync_all();
    }

    // --- global EDIT fields ------------------------------------------------

    pub(super) fn global_edit_text(&self, kind: Kind) -> String {
        let a = &self.snap.config.auto;
        let coord = |v: Option<f64>| v.map(|v| format!("{v:.2}")).unwrap_or_default();
        match kind {
            Kind::Lat => coord(a.latitude),
            Kind::Lon => coord(a.longitude),
            _ => self.auto.query.clone(),
        }
    }

    pub(super) fn ensure_global_edits(&mut self) {
        for kind in [Kind::Lat, Kind::Lon, Kind::Search] {
            if !self
                .edits
                .iter()
                .any(|e| e.mon.is_empty() && e.kind == kind)
            {
                if let Some(e) = self.make_edit("", kind) {
                    self.edits.push(e);
                }
            }
        }
    }

    /// A global EDIT lost focus: the search text is kept, a coordinate becomes an action
    /// if it is a number in range (rounded to 2 decimals); anything else is reverted.
    pub(super) fn commit_global(&mut self, edit: HWND) -> Option<Action> {
        let kind = self.edits.iter().find(|e| e.hwnd == edit)?.kind;
        let text = get_text(edit).trim().to_string();
        let a = &self.snap.config.auto;
        let (limit, current) = match kind {
            Kind::Lat => (90.0, a.latitude),
            Kind::Lon => (180.0, a.longitude),
            _ => {
                self.auto.query = text;
                return None;
            }
        };
        let value = text
            .replace(',', ".")
            .parse::<f64>()
            .ok()
            .filter(|v| v.abs() <= limit)
            .map(round2);
        match value {
            Some(v) if Some(v) != current => Some(Action::Auto(if kind == Kind::Lat {
                AutoEdit::Latitude(v)
            } else {
                AutoEdit::Longitude(v)
            })),
            _ => {
                self.restore_text(edit);
                None
            }
        }
    }

    // --- painting ---------------------------------------------------------

    pub(super) fn draw_auto(&self, cv: &Canvas, c: AutoCtl, r: Rect, hot: bool) {
        let a = &self.snap.config.auto;
        let (pal, fonts, s) = (cv.pal, cv.fonts, cv.s);
        match c {
            AutoCtl::Enabled => cv.switch(r, a.enabled, self.theme.accent),
            AutoCtl::UseSensor => cv.switch(r, a.use_sensor, self.theme.accent),
            AutoCtl::Weather => {
                cv.switch(r, a.weather, self.theme.accent);
                if !self.auto_enabled(c) {
                    // Greyed out: veil the switch with the card colour.
                    cv.fill(r, (r.b - r.t) / 2.0, pal.card.0, 0.6);
                }
            }
            AutoCtl::Day | AutoCtl::Night => {
                let level = if c == AutoCtl::Day {
                    a.day_brightness
                } else {
                    a.night_brightness
                };
                let pressed = self.drag == Some(Drag::Level(c));
                let fracs = (0.0, f32::from(level) / 100.0);
                cv.slider(r, fracs, false, self.theme.accent, (hot, pressed));
            }
            AutoCtl::Override(_) | AutoCtl::Source(_) => {
                self.draw_segment(cv, r, &self.seg_label(c), self.seg_selected(c), hot)
            }
            AutoCtl::Search | AutoCtl::Map | AutoCtl::Adaptive => {
                self.draw_button(cv, r, &self.btn_label(c), hot)
            }
            AutoCtl::Place(k) => {
                let Some(p) = self.auto.places.get(k) else {
                    return;
                };
                if hot {
                    cv.fill(r, 0.0, pal.text, 0.06);
                }
                let name = Rect::new(r.l + PAD * s, r.t, r.r - (PAD + 92.0) * s, r.b);
                cv.text(&p.label(), &fonts.body, name, pal.text, 1.0);
                if let Some(code) = p.postcodes.first() {
                    let at = Rect::new(r.r - (PAD + 84.0) * s, r.t, r.r - PAD * s, r.b);
                    cv.text(code, &fonts.value, at, pal.text, 0.6);
                }
            }
        }
    }
}
