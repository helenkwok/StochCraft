//! Calendars in the window: the model overview lists the model's calendars, each with its name
//! and its periods (a start, an end and how it repeats), and when model time 0 is, which places
//! them. Servers, sources and pools name a calendar for their availability, arrivals and shifts
//! (`form`'s calendar lists). Every change is one engine command, so each undoes in one step.

use serde_json::{Value, json};

use crate::editor::section;
use crate::form;
use crate::i18n::{t, tf};
use crate::{App, theme};

/// The days of the week, as an iCalendar `BYDAY` writes them, and as the window shows them.
fn days() -> [(&'static str, &'static str); 7] {
    [
        ("MO", t("Mon")),
        ("TU", t("Tue")),
        ("WE", t("Wed")),
        ("TH", t("Thu")),
        ("FR", t("Fri")),
        ("SA", t("Sat")),
        ("SU", t("Sun")),
    ]
}

/// A repeat rule taken apart: how often, on which days, and the parts the window does not edit
/// (`INTERVAL`, `WKST`, `UNTIL`, `COUNT`), which are kept as they are.
#[derive(Debug, Default, PartialEq)]
struct Repeat {
    freq: String,
    days: Vec<String>,
    rest: Vec<String>,
}

impl Repeat {
    fn parse(rule: &str) -> Self {
        let mut r = Self::default();
        for part in rule.split(';').filter(|p| !p.is_empty()) {
            match part.split_once('=') {
                Some(("FREQ", f)) => r.freq = f.to_owned(),
                Some(("BYDAY", d)) => r.days = d.split(',').map(str::to_owned).collect(),
                _ => r.rest.push(part.to_owned()),
            }
        }
        r
    }

    fn text(&self) -> String {
        let mut parts = vec![format!("FREQ={}", self.freq)];
        parts.extend(self.rest.iter().cloned());
        if !self.days.is_empty() {
            // In the week's order, whatever order they were picked in.
            let ordered: Vec<&str> = days()
                .iter()
                .map(|(d, _)| *d)
                .filter(|d| self.days.iter().any(|x| x == d))
                .collect();
            parts.push(format!("BYDAY={}", ordered.join(",")));
        }
        parts.join(";")
    }
}

/// The date a new period starts on: that of model time 0 if set, else of the first period of
/// any calendar, else 1 January 2026.
fn first_date(model: &Value) -> String {
    let start = model["experiment"]["start"].as_str().map(str::to_owned);
    let first_event = || {
        model["calendars"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|c| c["events"].as_array().into_iter().flatten())
            .filter_map(|e| e["start"].as_str())
            .min()
            .map(str::to_owned)
    };
    start
        .or_else(first_event)
        .and_then(|s| s.get(..10).map(str::to_owned))
        .unwrap_or_else(|| "2026-01-01".to_owned())
}

/// Sets field `f` of period `i` in the periods being changed (`null` removes it).
fn put(changed: &mut Option<Vec<Value>>, events: &[Value], i: usize, f: &str, v: Value) {
    let mut all = changed.clone().unwrap_or_else(|| events.to_vec());
    if v.is_null() {
        if let Some(o) = all[i].as_object_mut() {
            o.remove(f);
        }
    } else {
        all[i][f] = v;
    }
    *changed = Some(all);
}

impl App {
    /// The model overview's calendars: when model time 0 is, then each calendar with its name
    /// and periods, and a way to add one.
    pub(crate) fn calendars_section(&mut self, ui: &mut egui::Ui, model: &Value) {
        section(ui, t("Calendars"));
        ui.label(theme::weak(
            ui,
            t("Periods when availability, arrivals or a pool's shifts change. Dates and times are written 2026-01-31T09:00:00."),
        ));
        let unit = self.time_unit_label();

        // When model time 0 is: empty, midnight of the day the first period starts.
        let start = model["experiment"]["start"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        let mut cx = form::Cx {
            time_unit: &unit,
            numbers: &self.numbers,
            drafts: &mut self.drafts,
            calendars: &[],
        };
        let label = t("Model time 0 is");
        let changed = ui
            .horizontal(|ui| {
                form::label_cell(ui, label, 26.0).on_hover_text(t(
                    "Leave empty to start at midnight of the day the first period starts.",
                ));
                form::text(ui, &mut cx, "calendars/start", label, &json!(start), 180.0)
            })
            .inner;
        if let Some(v) = changed {
            let text = v.as_str().unwrap_or("").trim().to_owned();
            let value = if text.is_empty() {
                Value::Null
            } else {
                json!(text)
            };
            self.execute("experiment.set", json!({ "start": value }));
        }

        for c in model["calendars"].as_array().cloned().unwrap_or_default() {
            self.calendar(ui, model, &c);
        }
        ui.add_space(6.0);
        if ui.add(theme::secondary(ui, t("Add calendar"))).clicked()
            && let Some(r) = self.execute("calendar.add", json!({}))
            && let Some(id) = r["id"].as_str()
        {
            self.info(tf("Added calendar {id}", &[("id", id)]));
        }
    }

    /// One calendar: its name, its periods, and buttons to add a period or remove it.
    fn calendar(&mut self, ui: &mut egui::Ui, model: &Value, c: &Value) {
        let id = c["id"].as_str().unwrap_or("").to_owned();
        let events = c["events"].as_array().cloned().unwrap_or_default();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(theme::strong(c["name"].as_str().unwrap_or(&id)));
            ui.label(theme::weak(ui, &id).monospace());
            if ui
                .add(egui::Button::new(
                    egui::RichText::new(t("Remove")).size(12.0),
                ))
                .on_hover_text(tf("Remove the calendar {id}", &[("id", &id)]))
                .clicked()
            {
                // The engine refuses while a block or pool still uses it, and says which.
                self.execute("calendar.remove", json!({ "id": id }));
            }
        });
        if let Some(n) = self.text_field(
            ui,
            &format!("calendar/{id}/name"),
            t("Calendar name"),
            c["name"].as_str().unwrap_or(""),
        ) {
            self.execute("calendar.set", json!({ "id": id, "name": n }));
        }

        let mut changed: Option<Vec<Value>> = None;
        let unit = self.time_unit_label();
        ui.indent(format!("calendar/{id}/events"), |ui| {
            for (i, e) in events.iter().enumerate() {
                let k = format!("calendar/{id}/{i}");
                let mut cx = form::Cx {
                    time_unit: &unit,
                    numbers: &self.numbers,
                    drafts: &mut self.drafts,
                    calendars: &[],
                };
                let n = i + 1;
                let start_label = tf("Period {n} starts", &[("n", &n.to_string())]);
                let end_label = tf("Period {n} ends", &[("n", &n.to_string())]);
                ui.horizontal(|ui| {
                    form::label_cell(ui, start_label.as_str(), 26.0);
                    if let Some(v) = form::text(
                        ui,
                        &mut cx,
                        &format!("{k}/start"),
                        &start_label,
                        &e["start"],
                        180.0,
                    ) {
                        put(&mut changed, &events, i, "start", v);
                    }
                });
                ui.horizontal(|ui| {
                    form::label_cell(ui, end_label.as_str(), 26.0);
                    if let Some(v) = form::text(
                        ui,
                        &mut cx,
                        &format!("{k}/end"),
                        &end_label,
                        &e["end"],
                        180.0,
                    ) {
                        put(&mut changed, &events, i, "end", v);
                    }
                });

                let rule = Repeat::parse(e["repeat"].as_str().unwrap_or(""));
                let repeat_label = tf("Period {n} repeats", &[("n", &n.to_string())]);
                let freqs = [
                    ("", t("Does not repeat")),
                    ("DAILY", t("Daily")),
                    ("WEEKLY", t("Weekly")),
                ];
                ui.horizontal(|ui| {
                    form::label_cell(ui, t("Repeats"), 26.0);
                    let shown = freqs
                        .iter()
                        .find(|(f, _)| *f == rule.freq)
                        .map_or(rule.freq.clone(), |(_, l)| (*l).to_owned());
                    craft_ui::ComboBox::from_id_salt(format!("{k}/repeat"))
                        .selected_text(shown)
                        .width(170.0)
                        .show_ui(ui, |ui| {
                            for (f, l) in freqs {
                                if ui.selectable_label(f == rule.freq, l).clicked()
                                    && f != rule.freq
                                {
                                    if f.is_empty() {
                                        put(&mut changed, &events, i, "repeat", Value::Null);
                                    } else {
                                        // Days of the week apply to a weekly repeat only.
                                        let days = if f == "WEEKLY" {
                                            rule.days.clone()
                                        } else {
                                            Vec::new()
                                        };
                                        let r = Repeat {
                                            freq: f.to_owned(),
                                            days,
                                            rest: rule.rest.clone(),
                                        };
                                        put(&mut changed, &events, i, "repeat", json!(r.text()));
                                    }
                                }
                            }
                        })
                        .response
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &repeat_label)
                        });
                    if ui
                        .small_button(t("Remove"))
                        .on_hover_text(t("Remove this period"))
                        .clicked()
                    {
                        let mut all = changed.clone().unwrap_or_else(|| events.clone());
                        all.remove(i);
                        changed = Some(all);
                    }
                });
                if rule.freq == "WEEKLY" {
                    ui.horizontal(|ui| {
                        form::label_cell(ui, t("On"), 26.0)
                            .on_hover_text(t("None picked: the day the period starts."));
                        for (d, name) in days() {
                            let on = rule.days.iter().any(|x| x == d);
                            if ui.selectable_label(on, name).clicked() {
                                let mut r = Repeat {
                                    freq: rule.freq.clone(),
                                    days: rule.days.clone(),
                                    rest: rule.rest.clone(),
                                };
                                if on {
                                    r.days.retain(|x| x != d);
                                } else {
                                    r.days.push(d.to_owned());
                                }
                                put(&mut changed, &events, i, "repeat", json!(r.text()));
                            }
                        }
                    });
                }
                if !rule.rest.is_empty() {
                    ui.label(theme::weak(
                        ui,
                        tf("Also: {rule}", &[("rule", &rule.rest.join(";"))]),
                    ));
                }
                ui.add_space(4.0);
            }
            if ui.small_button(t("Add period")).clicked() {
                let date = first_date(model);
                let mut all = changed.clone().unwrap_or_else(|| events.clone());
                all.push(json!({
                    "start": format!("{date}T09:00:00"),
                    "end": format!("{date}T17:00:00"),
                    "repeat": "FREQ=DAILY",
                }));
                changed = Some(all);
            }
        });
        if let Some(all) = changed {
            self.execute("calendar.set", json!({ "id": id, "events": all }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Repeat;

    #[test]
    fn a_repeat_rule_keeps_the_parts_the_window_does_not_edit() {
        let r = Repeat::parse("FREQ=WEEKLY;INTERVAL=2;BYDAY=FR,MO;COUNT=10");
        assert_eq!(r.freq, "WEEKLY");
        assert_eq!(r.days, ["FR", "MO"]);
        assert_eq!(r.rest, ["INTERVAL=2", "COUNT=10"]);
        // Days in the week's order.
        assert_eq!(r.text(), "FREQ=WEEKLY;INTERVAL=2;COUNT=10;BYDAY=MO,FR");
        assert_eq!(Repeat::parse("FREQ=DAILY").text(), "FREQ=DAILY");
    }
}
