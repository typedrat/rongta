//! Opening a template, filling in its fields, and previewing the result.
use crate::{
    errors::{Problem, problem_ui},
    preview::{HEAD_DOTS, Preview},
    theme,
    worker::Job,
};
use eframe::egui;
use rongta_template::{Template, Values};
use std::{collections::BTreeMap, path::PathBuf};

/// Layout failures whose message already names, in plain words, the text
/// that has to change.
const PLAIN_LAYOUT_FAILURES: [&str; 3] = [
    "too long to fit",
    "which the label font cannot print",
    "too long to encode",
];

#[derive(Default)]
pub struct TemplateTab {
    template: Option<(PathBuf, Template)>,
    values: Values,
    load_problem: Option<Problem>,
    /// Keyed by the normalized values; opening a template clears it.
    preview: Preview<Values>,
}

fn render_problem(error: &anyhow::Error) -> Problem {
    let cause = error.root_cause().to_string();
    let summary = if PLAIN_LAYOUT_FAILURES.iter().any(|s| cause.contains(s)) {
        format!("This label can't be drawn: {cause}.")
    } else {
        "This label can't be drawn.".to_owned()
    };
    Problem::new(summary, error)
}

impl TemplateTab {
    pub fn open(&mut self, path: PathBuf) {
        match Template::load(&path) {
            Ok(template) => {
                self.values = template
                    .fields
                    .iter()
                    .map(|field| (field.id.clone(), String::new()))
                    .collect();
                self.template = Some((path, template));
                self.load_problem = None;
                self.preview.clear();
            }
            Err(error) => {
                self.load_problem = Some(Problem::new(
                    "This template can't be used. Ask whoever sent it for a new copy.",
                    &error,
                ))
            }
        }
    }

    /// A job only when the label can actually be drawn: `check` can't see
    /// layout failures such as text too long for its column. Rendering one
    /// small label takes well under a millisecond, so doing it per frame is
    /// fine. Otherwise, a short reason Print is unavailable.
    pub fn job(&self) -> Result<Job, &'static str> {
        let (_, template) = self.template.as_ref().ok_or("Open a template to print")?;
        let values = template.normalize(&self.values);
        if !template.check(&values).is_empty() {
            return Err("Fill in the form to print");
        }
        if template.render(&values).is_err() {
            return Err("Change what the preview points out to print");
        }
        Ok(Job::Template {
            template: template.clone(),
            values,
        })
    }

    #[cfg(test)]
    pub fn set_value(&mut self, id: &str, value: &str) {
        self.values.insert(id.into(), value.into());
    }

    /// Errors for fields the person has typed in; blank fields just aren't
    /// ready yet.
    pub fn visible_errors(&self) -> BTreeMap<String, String> {
        let Some((_, template)) = &self.template else {
            return BTreeMap::new();
        };
        let values = template.normalize(&self.values);
        template
            .check(&values)
            .into_iter()
            .filter(|error| values.get(&error.field).is_some_and(|v| !v.is_empty()))
            .map(|error| (error.field, error.message))
            .collect()
    }

    /// Returns whether the person changed anything: typed, or opened a file.
    pub fn form_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let p = theme::palette(ui);
        let errors = self.visible_errors();
        let mut edited = false;
        theme::card(ui, |ui| {
            ui.horizontal(|ui| {
                let title = self
                    .template
                    .as_ref()
                    .map(|(_, t)| t.name.as_str())
                    .unwrap_or("No template open");
                ui.heading(title);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Open…").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("Label template", &["toml"])
                            .pick_file()
                    {
                        self.open(path);
                        edited = true;
                    }
                });
            });
            if let Some(problem) = &self.load_problem {
                problem_ui(ui, problem);
            }
            let Some((_, template)) = &self.template else {
                ui.label(
                    egui::RichText::new(
                        "Open the template file you were sent, or drop it on this window.",
                    )
                    .color(p.muted),
                );
                return;
            };
            ui.add_space(4.0);
            for field in &template.fields {
                let label = ui.label(
                    egui::RichText::new(&field.label)
                        .font(egui::FontId::new(14.0, theme::semibold())),
                );
                let value = self.values.entry(field.id.clone()).or_default();
                edited |= ui
                    .add(
                        egui::TextEdit::singleline(value)
                            .desired_width(f32::INFINITY)
                            .font(egui::FontId::proportional(18.0))
                            .margin(egui::Margin::symmetric(10, 8)),
                    )
                    .labelled_by(label.id)
                    .changed();
                match (errors.get(&field.id), &field.hint) {
                    (Some(error), _) => {
                        ui.label(egui::RichText::new(error).small().color(p.error));
                    }
                    (None, Some(hint)) => {
                        ui.label(egui::RichText::new(hint).small().color(p.muted));
                    }
                    (None, None) => {}
                }
                ui.add_space(6.0);
            }
        });
        edited
    }

    pub fn preview_ui(&mut self, ui: &mut egui::Ui) {
        let Some((_, template)) = &self.template else {
            self.preview.show(ui, "Preview");
            return;
        };
        let values = template.normalize(&self.values);
        if template.check(&values).is_empty() {
            self.preview.update(ui.ctx(), &values, || {
                template
                    .raster(&values, HEAD_DOTS)
                    .map_err(|error| render_problem(&error))
            });
        } else {
            self.preview.clear();
        }
        self.preview
            .show(ui, &format!("Preview · {} label", template.media()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> PathBuf {
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../template/examples/access-code.rongta.toml"
        ))
    }

    #[test]
    fn job_is_ready_only_when_every_field_is_valid() {
        let mut tab = TemplateTab::default();
        assert_eq!(tab.job().err(), Some("Open a template to print"));
        tab.open(example());
        assert_eq!(tab.job().err(), Some("Fill in the form to print"));
        // An empty field is incomplete, not an error worth showing yet.
        assert!(tab.visible_errors().is_empty());

        tab.values.insert("code".into(), "ab1".into());
        assert!(tab.job().is_err());
        assert_eq!(tab.visible_errors()["code"], "6 to 12 letters or digits");

        tab.values.insert("code".into(), "ab12cd".into());
        let Ok(Job::Template { values, .. }) = tab.job() else {
            panic!("expected a template job");
        };
        assert_eq!(values["code"], "AB12CD");
    }

    #[test]
    fn layout_failures_keep_their_plain_message() {
        let error = anyhow::anyhow!("\"ABCDEFGHIJKL\" is too long to fit beside the QR code")
            .context("Cannot draw the label");
        let problem = render_problem(&error);
        assert_eq!(
            problem.summary,
            "This label can't be drawn: \"ABCDEFGHIJKL\" is too long to fit beside the QR code."
        );
        assert_eq!(problem.details, format!("{error:#}"));

        let problem = render_problem(&anyhow::anyhow!("raster stride mismatch"));
        assert_eq!(problem.summary, "This label can't be drawn.");
    }

    #[test]
    fn a_bad_file_reports_why_and_keeps_the_previous_template() {
        let mut tab = TemplateTab::default();
        tab.open(example());
        tab.open(PathBuf::from("/nonexistent/template.rongta.toml"));
        let problem = tab.load_problem.as_ref().unwrap();
        assert!(problem.summary.starts_with("This template can't be used."));
        assert!(problem.details.contains("Cannot read template"));
        assert!(tab.template.is_some());
    }
}
