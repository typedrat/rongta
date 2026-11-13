//! Opening a template, filling in its fields, and previewing the result.
use crate::{preview::Preview, theme, worker::Job};
use eframe::egui;
use rongta_template::{Template, Values};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Default)]
pub struct TemplateTab {
    template: Option<(PathBuf, Template)>,
    values: Values,
    load_error: Option<String>,
    preview: Preview,
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
                self.load_error = None;
                self.preview.clear();
            }
            Err(error) => self.load_error = Some(format!("{error:#}")),
        }
    }

    /// A job only when the label can actually be drawn: `check` can't see
    /// layout failures such as text too long for its column. Rendering one
    /// small label takes well under a millisecond, so doing it per frame is
    /// fine.
    pub fn job(&self) -> Option<Job> {
        let (_, template) = self.template.as_ref()?;
        let values = template.normalize(&self.values);
        (template.check(&values).is_empty() && template.render(&values).is_ok()).then(|| {
            Job::Template {
                template: template.clone(),
                values,
            }
        })
    }

    /// Errors for fields the person has typed in; blank fields just aren't
    /// ready yet.
    #[cfg(test)]
    pub fn set_value(&mut self, id: &str, value: &str) {
        self.values.insert(id.into(), value.into());
    }

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

    pub fn form_ui(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui);
        let errors = self.visible_errors();
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
                    }
                });
            });
            if let Some(error) = &self.load_error {
                ui.label(egui::RichText::new(error).color(p.error));
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
                ui.add(
                    egui::TextEdit::singleline(value)
                        .desired_width(f32::INFINITY)
                        .font(egui::FontId::proportional(18.0))
                        .margin(egui::Margin::symmetric(10, 8)),
                )
                .labelled_by(label.id);
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
    }

    pub fn preview_ui(&mut self, ui: &mut egui::Ui) {
        let Some((path, template)) = &self.template else {
            self.preview.show(ui, "Preview");
            return;
        };
        let caption = format!("Preview · {} label", template.media());
        let values = template.normalize(&self.values);
        if template.check(&values).is_empty() {
            let key = format!("{}{values:?}", path.display());
            let template = template.clone();
            self.preview
                .update(ui.ctx(), key, move || template.raster(&values, 384));
        } else {
            self.preview.clear();
        }
        self.preview.show(ui, &caption);
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
        assert!(tab.job().is_none());
        tab.open(example());
        assert!(tab.job().is_none());
        // An empty field is incomplete, not an error worth showing yet.
        assert!(tab.visible_errors().is_empty());

        tab.values.insert("code".into(), "ab1".into());
        assert!(tab.job().is_none());
        assert_eq!(tab.visible_errors()["code"], "6 to 12 letters or digits");

        tab.values.insert("code".into(), "ab12cd".into());
        let Some(Job::Template { values, .. }) = tab.job() else {
            panic!("expected a template job");
        };
        assert_eq!(values["code"], "AB12CD");
    }

    #[test]
    fn a_bad_file_reports_why_and_keeps_the_previous_template() {
        let mut tab = TemplateTab::default();
        tab.open(example());
        tab.open(PathBuf::from("/nonexistent/template.rongta.toml"));
        assert!(
            tab.load_error
                .as_deref()
                .unwrap()
                .contains("Cannot read template")
        );
        assert!(tab.template.is_some());
    }
}
