//! The start page (#149, screen 4a): "What do you want to find out?". Examples by question and
//! industry, each card with its question, what it teaches, how long it takes, a strip of its
//! blocks and a proof tag where its answer is checked; beside them, three doors: one for people
//! new to simulation, one for people coming from another simulation tool, and opening a file.
//! No file formats or acronyms on this page.

use crate::i18n::{t, tf};
use crate::theme;

/// What an example asks, for the question filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topic {
    Bottleneck,
    Staffing,
    Change,
    Breakdown,
}

impl Topic {
    pub const ALL: [Topic; 4] = [
        Topic::Bottleneck,
        Topic::Staffing,
        Topic::Change,
        Topic::Breakdown,
    ];

    fn label(self) -> &'static str {
        match self {
            Topic::Bottleneck => t("Where is the bottleneck?"),
            Topic::Staffing => t("How many people or machines?"),
            Topic::Change => t("Will a change help?"),
            Topic::Breakdown => t("Why did it break down?"),
        }
    }
}

/// An example's field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Industry {
    Services,
    Manufacturing,
    Healthcare,
    ContactCentre,
    ItOperations,
    /// Projects and their schedules.
    Projects,
    /// Not an industry: models for learning the tool itself.
    Reference,
}

impl Industry {
    pub const ALL: [Industry; 7] = [
        Industry::Services,
        Industry::Manufacturing,
        Industry::Healthcare,
        Industry::ContactCentre,
        Industry::ItOperations,
        Industry::Projects,
        Industry::Reference,
    ];

    fn label(self) -> &'static str {
        match self {
            Industry::Services => t("Services"),
            Industry::Manufacturing => t("Manufacturing"),
            Industry::Healthcare => t("Healthcare"),
            Industry::ContactCentre => t("Contact centre"),
            Industry::ItOperations => t("IT operations"),
            Industry::Projects => t("Projects"),
            Industry::Reference => t("Reference"),
        }
    }
}

/// How an example's answer was checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proof {
    /// Against an exact formula from queueing theory.
    Theory,
    /// Against an exact result of probability theory (not a queue).
    Probability,
    /// Against the figures of a published study.
    Published,
}

impl Proof {
    fn label(self) -> &'static str {
        match self {
            Proof::Theory => t("Checked against queueing theory"),
            Proof::Probability => t("Checked against probability theory"),
            Proof::Published => t("Matches a published study"),
        }
    }
}

/// A block in a card's strip: the shape of its kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Arrivals,
    Task,
    Resource,
    Shared,
    Decision,
    Split,
    Departures,
}

/// What the start page shows of an example: its card. The model itself is in
/// [`crate::EXAMPLES`].
pub struct Card {
    pub question: &'static str,
    pub industry: Industry,
    pub topics: &'static [Topic],
    pub teaches: &'static str,
    /// About how many minutes it takes to open, run and read.
    pub minutes: u8,
    pub proof: Option<Proof>,
    pub flow: &'static [Glyph],
    /// Shown before "See all examples".
    pub featured: bool,
}

/// The start page's choices.
#[derive(Default)]
pub struct Start {
    pub topic: Option<Topic>,
    pub industry: Option<Industry>,
    /// Every example, not only the featured ones.
    pub all: bool,
}

/// What the start page asks the app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open example `i` of [`crate::EXAMPLES`].
    Example(usize),
    /// Start an empty model with Getting started beside it.
    FirstModel,
    /// Read the guide for people coming from another tool.
    Switching,
    Open,
    /// Open recent file `i`.
    Recent(usize),
    Blank,
}

/// Whether card `c` passes the filters of `start`.
fn shown(c: &Card, start: &Start) -> bool {
    (start.all || c.featured)
        && start.topic.is_none_or(|q| c.topics.contains(&q))
        && start.industry.is_none_or(|i| c.industry == i)
}

/// Draws the start page. `cards` are the examples' cards in [`crate::EXAMPLES`] order; `recent`
/// the recent files' names (none in the browser); `open_key` the Open shortcut as written.
pub fn show(
    ui: &mut egui::Ui,
    start: &mut Start,
    cards: &[&Card],
    recent: &[String],
    open_key: &str,
) -> Option<Action> {
    let p = theme::palette(ui.ctx());
    let mut action = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_max_width(ui.available_width().min(1200.0));
            ui.add_space(12.0);
            ui.label(theme::strong(t("What do you want to find out?")).size(26.0));
            ui.add_space(6.0);
            ui.add(
                egui::Label::new(theme::weak(
                    ui,
                    t("Simulate a process, compare what-ifs, and see which differences are real. Each example asks one question and shows how the answer was found."),
                ))
                .wrap(),
            );
            ui.add_space(14.0);
            filters(ui, start);
            ui.add_space(12.0);
            let wide = ui.available_width() >= 980.0;
            if wide {
                ui.horizontal_top(|ui| {
                    let side = 340.0;
                    let gallery = ui.available_width() - side - 64.0;
                    ui.vertical(|ui| {
                        ui.set_width(gallery);
                        examples(ui, &p, start, cards, &mut action);
                    });
                    ui.add_space(16.0);
                    ui.separator();
                    ui.add_space(16.0);
                    ui.vertical(|ui| {
                        ui.set_width(side);
                        doors(ui, recent, open_key, &mut action);
                    });
                });
            } else {
                examples(ui, &p, start, cards, &mut action);
                ui.add_space(20.0);
                doors(ui, recent, open_key, &mut action);
            }
            ui.add_space(20.0);
        });
    action
}

/// The question chips and the industry choice.
fn filters(ui: &mut egui::Ui, start: &mut Start) {
    ui.horizontal_wrapped(|ui| {
        ui.label(theme::weak(ui, t("Questions")));
        if ui
            .selectable_label(start.topic.is_none(), t("All"))
            .clicked()
        {
            start.topic = None;
        }
        for q in Topic::ALL {
            if ui
                .selectable_label(start.topic == Some(q), q.label())
                .clicked()
            {
                start.topic = (start.topic != Some(q)).then_some(q);
            }
        }
        ui.add_space(12.0);
        let label = ui.label(theme::weak(ui, t("Industry")));
        let chosen = start.industry.map_or(t("All industries"), Industry::label);
        egui::ComboBox::from_id_salt("start-industry")
            .selected_text(chosen)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut start.industry, None, t("All industries"));
                for i in Industry::ALL {
                    ui.selectable_value(&mut start.industry, Some(i), i.label());
                }
            })
            .response
            .labelled_by(label.id);
    });
}

/// The example cards, two to a row where they fit, then "See all examples".
fn examples(
    ui: &mut egui::Ui,
    p: &theme::Palette,
    start: &mut Start,
    cards: &[&Card],
    action: &mut Option<Action>,
) {
    let list: Vec<(usize, &&Card)> = cards
        .iter()
        .enumerate()
        .filter(|(_, c)| shown(c, start))
        .collect();
    if list.is_empty() {
        ui.label(theme::weak(ui, t("No example matches these choices yet.")));
    }
    let gap = 14.0;
    let per_row = if ui.available_width() >= 640.0 { 2 } else { 1 };
    let width = (ui.available_width() - gap * (per_row as f32 - 1.0)) / per_row as f32;
    for row in list.chunks(per_row) {
        ui.horizontal_top(|ui| {
            for (i, c) in row {
                if card(ui, p, c, width) {
                    *action = Some(Action::Example(*i));
                }
                ui.add_space(gap);
            }
        });
        ui.add_space(gap);
    }
    let hidden = cards.iter().filter(|c| !c.featured).count();
    if hidden > 0 {
        let label = if start.all {
            t("Show the featured examples only").to_owned()
        } else {
            tf(
                "See all {n} examples, including reference models",
                &[("n", &cards.len().to_string())],
            )
        };
        if ui.link(label).clicked() {
            start.all = !start.all;
        }
    }
}

/// One example's card; true when clicked (or chosen from the keyboard).
fn card(ui: &mut egui::Ui, p: &theme::Palette, c: &Card, width: f32) -> bool {
    let inner = egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, p.border))
        .fill(p.raised)
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(16, 14))
        // Top to bottom inside, although the cards sit side by side.
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_width(width - 34.0);
                // The whole card is the button: its text is not selectable, so a click on the words
                // opens the example too.
                ui.style_mut().interaction.selectable_labels = false;
                // A row of fixed height: in the scrolling page a right-aligned part would otherwise
                // be centred in all the height there is.
                let row = egui::vec2(ui.available_width(), 18.0);
                // The proof goes at the right of the row when it fits beside the industry and
                // time, else on a row of its own under them, so the two never overlap.
                let proof_width = |ui: &egui::Ui, proof: Proof| {
                    let text = ui
                        .painter()
                        .layout_no_wrap(
                            proof.label().to_owned(),
                            egui::FontId::proportional(12.0),
                            p.text,
                        )
                        .size()
                        .x;
                    text + 14.0 + 2.0 * ui.spacing().item_spacing.x
                };
                let proof_row = |ui: &mut egui::Ui, proof: Proof| {
                    ui.label(egui::RichText::new(proof.label()).size(12.0));
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                    theme::paint_check_mark(ui.painter(), rect.center(), 5.0, p);
                };
                let mut below = None;
                ui.allocate_ui_with_layout(
                    row,
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} \u{b7} {}",
                                c.industry.label(),
                                tf("{n} min", &[("n", &c.minutes.to_string())])
                            ))
                            .size(12.0)
                            .color(p.accent),
                        );
                        if let Some(proof) = c.proof {
                            if ui.available_width() >= proof_width(ui, proof) {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| proof_row(ui, proof),
                                );
                            } else {
                                below = Some(proof);
                            }
                        }
                    },
                );
                if let Some(proof) = below {
                    ui.allocate_ui_with_layout(
                        row,
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| proof_row(ui, proof),
                    );
                }
                ui.add_space(4.0);
                ui.add(egui::Label::new(theme::strong(t(c.question)).size(16.0)).wrap());
                // A model with no blocks (a Monte Carlo model) has no flow to draw.
                if !c.flow.is_empty() {
                    ui.add_space(8.0);
                    strip(ui, p, c.flow);
                }
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(theme::strong(t("Teaches:")));
                    ui.label(t(c.teaches));
                });
            })
        });
    let rect = inner.response.rect;
    let r = ui.interact(
        rect,
        ui.make_persistent_id(c.question),
        egui::Sense::click(),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, t(c.question)));
    if r.hovered() || r.has_focus() {
        ui.painter().rect_stroke(
            rect,
            8.0,
            egui::Stroke::new(1.5, p.accent),
            egui::StrokeKind::Inside,
        );
    }
    let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
    r.clicked()
}

/// A card's strip of block shapes, joined by short lines.
fn strip(ui: &mut egui::Ui, p: &theme::Palette, flow: &[Glyph]) {
    let size = 18.0;
    let link = 10.0;
    let width = flow.len() as f32 * (size + link);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, size + 4.0), egui::Sense::hover());
    let painter = ui.painter_at(rect.expand(2.0));
    let ink = egui::Stroke::new(1.5, p.text);
    let y = rect.center().y;
    for (i, g) in flow.iter().enumerate() {
        let x = rect.left() + i as f32 * (size + link) + size / 2.0;
        let c = egui::pos2(x, y);
        if i > 0 {
            painter.line_segment(
                [
                    egui::pos2(x - size / 2.0 - link, y),
                    egui::pos2(x - size / 2.0, y),
                ],
                egui::Stroke::new(1.0, p.text_weak),
            );
        }
        let r = egui::Rect::from_center_size(c, egui::vec2(size, size * 0.72));
        match g {
            Glyph::Arrivals => {
                painter.circle_stroke(c, size / 2.0 - 1.0, ink);
            }
            Glyph::Departures => {
                painter.circle_stroke(c, size / 2.0 - 1.5, egui::Stroke::new(3.0, p.text));
            }
            Glyph::Task | Glyph::Resource | Glyph::Shared => {
                painter.rect_stroke(r, 3.0, ink, egui::StrokeKind::Inside);
                let dots = match g {
                    Glyph::Resource => 2,
                    Glyph::Shared => 3,
                    _ => 0,
                };
                for d in 0..dots {
                    let at = egui::pos2(r.left() + 4.0 + d as f32 * 3.5, r.bottom() - 4.0);
                    painter.rect_filled(
                        egui::Rect::from_center_size(at, egui::vec2(2.2, 2.2)),
                        0.0,
                        p.text,
                    );
                }
            }
            Glyph::Decision | Glyph::Split => {
                let h = size / 2.0;
                let points = vec![
                    c + egui::vec2(0.0, -h),
                    c + egui::vec2(h, 0.0),
                    c + egui::vec2(0.0, h),
                    c + egui::vec2(-h, 0.0),
                ];
                painter.add(egui::Shape::closed_line(points, ink));
                let k = h * 0.45;
                if *g == Glyph::Decision {
                    painter.line_segment([c + egui::vec2(-k, -k), c + egui::vec2(k, k)], ink);
                    painter.line_segment([c + egui::vec2(-k, k), c + egui::vec2(k, -k)], ink);
                } else {
                    painter.line_segment([c + egui::vec2(-k, 0.0), c + egui::vec2(k, 0.0)], ink);
                    painter.line_segment([c + egui::vec2(0.0, -k), c + egui::vec2(0.0, k)], ink);
                }
            }
        }
    }
}

/// The three doors beside the examples.
fn doors(ui: &mut egui::Ui, recent: &[String], open_key: &str, action: &mut Option<Action>) {
    let p = theme::palette(ui.ctx());
    ui.label(theme::strong(t("New to simulation?")).size(17.0));
    ui.add_space(4.0);
    ui.add(
        egui::Label::new(t(
            "Build a first model in five short steps on the real canvas: arrivals, a task, departures, run, then read the finding. Getting started ticks each step as you go.",
        ))
        .wrap(),
    );
    ui.add_space(8.0);
    if ui
        .add(theme::primary(ui, t("Build a first model")))
        .clicked()
    {
        *action = Some(Action::FirstModel);
    }
    ui.add_space(14.0);
    ui.separator();
    ui.add_space(10.0);
    ui.label(theme::strong(t("Coming from another simulation tool?")).size(17.0));
    ui.add_space(4.0);
    ui.add(
        egui::Label::new(t(
            "How blocks, runs and confidence intervals work here, which keys do what, and how to bring in a process diagram you already have.",
        ))
        .wrap(),
    );
    ui.add_space(8.0);
    if ui
        .add(theme::secondary(ui, t("Read the two-page guide")))
        .clicked()
    {
        *action = Some(Action::Switching);
    }
    ui.add_space(14.0);
    ui.separator();
    ui.add_space(10.0);
    ui.label(theme::strong(t("Open a file")).size(17.0));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.add(theme::secondary(ui, t("Open a file…"))).clicked() {
            *action = Some(Action::Open);
        }
        ui.label(theme::weak(ui, open_key));
    });
    if !recent.is_empty() {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(t("Recent"))
                .size(12.0)
                .color(p.text_weak),
        );
        for (i, name) in recent.iter().enumerate().take(5) {
            if ui.link(name).clicked() {
                *action = Some(Action::Recent(i));
            }
        }
    }
    ui.add_space(8.0);
    if ui.link(t("Start from a blank model")).clicked() {
        *action = Some(Action::Blank);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(featured: bool, industry: Industry, topics: &'static [Topic]) -> Card {
        Card {
            question: "Q",
            industry,
            topics,
            teaches: "T",
            minutes: 2,
            proof: None,
            flow: &[],
            featured,
        }
    }

    #[test]
    fn the_filters_narrow_the_cards() {
        let a = card(true, Industry::Services, &[Topic::Staffing]);
        let b = card(
            true,
            Industry::Manufacturing,
            &[Topic::Bottleneck, Topic::Breakdown],
        );
        let c = card(false, Industry::Reference, &[]);
        let mut start = Start::default();
        assert!(shown(&a, &start) && shown(&b, &start) && !shown(&c, &start));
        start.topic = Some(Topic::Breakdown);
        assert!(!shown(&a, &start) && shown(&b, &start));
        start.topic = None;
        start.industry = Some(Industry::Services);
        assert!(shown(&a, &start) && !shown(&b, &start));
        start.industry = None;
        start.all = true;
        assert!(shown(&c, &start));
    }
}
