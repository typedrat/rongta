//! Opening a template, filling in its fields, and previewing the result.
use crate::{
    errors::{Problem, problem_ui},
    preview::{HEAD_DOTS, Preview},
    theme,
    worker::Job,
};
use eframe::egui;
use rongta_template::{Template, Values};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Layout failures whose message already names, in plain words, the text
/// that has to change.
const PLAIN_LAYOUT_FAILURES: [&str; 4] = [
    "too long to fit",
    "which the label font cannot print",
    "too long to encode",
    "too many to scan",
];

#[derive(Default)]
pub struct TemplateTab {
    template: Option<(PathBuf, Template)>,
    values: Values,
    load_problem: Option<Problem>,
    /// Keyed by the normalized values; opening a template clears it.
    preview: Preview<Values>,
    /// Where opened templates are copied, so the app can reopen them after
    /// the original is moved or deleted. `None` keeps no copies.
    copies: Option<PathBuf>,
    /// The template to reopen at the next start.
    remembered: Option<PathBuf>,
    /// A muted aside about the copy or the remembered template.
    note: Option<String>,
}

fn render_problem(error: &anyhow::Error) -> Problem {
    // The plain sentence is sometimes the outermost context, as for QR
    // content over a library error, and sometimes the root cause, so the
    // whole chain is searched.
    let plain = error
        .chain()
        .map(ToString::to_string)
        .find(|layer| PLAIN_LAYOUT_FAILURES.iter().any(|s| layer.contains(s)));
    let summary = match plain {
        Some(message) => format!("This label can't be drawn: {message}."),
        None => "This label can't be drawn.".to_owned(),
    };
    Problem::new(summary, error)
}

/// Copies `source` into `copies` under its own name, replacing an older copy,
/// and returns the copy's path.
fn keep_copy(copies: &Path, source: &Path) -> std::io::Result<PathBuf> {
    let name = source
        .file_name()
        .ok_or_else(|| std::io::Error::other("the template path has no file name"))?;
    let copy = copies.join(name);
    // A template reopened at start is already the copy.
    if let (Ok(source), Ok(existing)) = (source.canonicalize(), copy.canonicalize())
        && source == existing
    {
        return Ok(copy);
    }
    std::fs::create_dir_all(copies)?;
    // Written beside the copy and renamed over it, so a crash part way
    // through never leaves a half-written template to reopen.
    let mut temp = tempfile::NamedTempFile::new_in(copies)?;
    std::io::copy(&mut std::fs::File::open(source)?, &mut temp)?;
    temp.persist(&copy)?;
    Ok(copy)
}

impl TemplateTab {
    pub fn new(copies: Option<PathBuf>, remembered: Option<PathBuf>) -> Self {
        Self {
            copies,
            remembered,
            ..Self::default()
        }
    }

    pub fn remembered(&self) -> Option<&Path> {
        self.remembered.as_deref()
    }

    /// Opens the template remembered from last time. If that fails, the
    /// window starts empty with a note rather than an error: the person did
    /// nothing wrong, and has only to open the template again.
    pub fn reopen(&mut self, path: PathBuf) {
        match Template::load(&path) {
            Ok(template) => self.show(path, template),
            Err(_) => {
                let name = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                );
                self.note = Some(format!("Couldn't reopen {name} — open the template again."));
                self.remembered = None;
            }
        }
    }

    pub fn open(&mut self, path: PathBuf) {
        match Template::load(&path) {
            Ok(template) => self.show(path, template),
            Err(error) => {
                self.load_problem = Some(Problem::new(
                    "This template can't be used. Ask whoever sent it for a new copy.",
                    &error,
                ))
            }
        }
    }

    /// Shows a template that loaded, and remembers it for next time. A
    /// failed copy is only noted: the template is open and prints the same,
    /// it just won't reopen if the original goes away.
    fn show(&mut self, path: PathBuf, template: Template) {
        let (remembered, note) = match &self.copies {
            Some(copies) => match keep_copy(copies, &path) {
                Ok(copy) => (copy, None),
                Err(_) => (
                    path.clone(),
                    Some(
                        "Couldn't keep a copy of this template to reopen next time. \
                         Printing still works."
                            .to_owned(),
                    ),
                ),
            },
            None => (path.clone(), None),
        };
        self.remembered = Some(remembered);
        self.note = note;
        self.values = template
            .fields
            .iter()
            .map(|field| (field.id.clone(), String::new()))
            .collect();
        self.template = Some((path, template));
        self.load_problem = None;
        self.preview.clear();
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
            if let Some(note) = &self.note {
                ui.add(egui::Label::new(egui::RichText::new(note).color(p.muted)).wrap());
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

    fn example_in(dir: &Path) -> PathBuf {
        let path = dir.join("access-code.rongta.toml");
        std::fs::copy(example(), &path).unwrap();
        path
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn opening_keeps_a_copy_and_remembers_it() {
        let originals = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let copies = data.path().join("templates");
        let original = example_in(originals.path());

        let mut tab = TemplateTab::new(Some(copies.clone()), None);
        tab.open(original.clone());
        let copy = copies.join("access-code.rongta.toml");
        assert!(tab.template.is_some());
        assert_eq!(tab.remembered(), Some(copy.as_path()));
        assert_eq!(
            std::fs::read(&copy).unwrap(),
            std::fs::read(&original).unwrap()
        );
        assert_eq!(tab.note, None);
    }

    #[test]
    fn opening_replaces_an_older_copy() {
        let originals = tempfile::tempdir().unwrap();
        let copies = tempfile::tempdir().unwrap();
        let original = example_in(originals.path());
        let copy = copies.path().join("access-code.rongta.toml");
        std::fs::write(&copy, "an older version").unwrap();

        let mut tab = TemplateTab::new(Some(copies.path().into()), None);
        tab.open(original.clone());
        assert_eq!(
            std::fs::read(&copy).unwrap(),
            std::fs::read(&original).unwrap()
        );
        assert_eq!(names(copies.path()), ["access-code.rongta.toml"]);
    }

    #[test]
    fn reopening_the_copy_leaves_it_alone() {
        let copies = tempfile::tempdir().unwrap();
        let copy = example_in(copies.path());
        let modified = std::fs::metadata(&copy).unwrap().modified().unwrap();

        let mut tab = TemplateTab::new(Some(copies.path().into()), Some(copy.clone()));
        tab.reopen(copy.clone());
        assert!(tab.template.is_some());
        assert_eq!(tab.remembered(), Some(copy.as_path()));
        assert_eq!(
            std::fs::metadata(&copy).unwrap().modified().unwrap(),
            modified
        );
        assert_eq!(names(copies.path()), ["access-code.rongta.toml"]);
        assert_eq!(tab.note, None);
    }

    #[test]
    fn a_failed_copy_still_opens_and_remembers_the_original() {
        let originals = tempfile::tempdir().unwrap();
        let original = example_in(originals.path());
        // A file where the folder should be, so the copy can't be written.
        let blocked = originals.path().join("templates");
        std::fs::write(&blocked, "").unwrap();

        let mut tab = TemplateTab::new(Some(blocked), None);
        tab.open(original.clone());
        assert!(tab.template.is_some());
        assert!(tab.load_problem.is_none());
        assert_eq!(tab.remembered(), Some(original.as_path()));
        assert!(tab.note.is_some());
        assert!(
            tab.job()
                .is_err_and(|why| why == "Fill in the form to print")
        );
    }

    #[test]
    fn a_missing_remembered_template_is_noted_and_forgotten() {
        let copies = tempfile::tempdir().unwrap();
        let missing = copies.path().join("access-code.rongta.toml");

        let mut tab = TemplateTab::new(Some(copies.path().into()), Some(missing.clone()));
        tab.reopen(missing);
        assert!(tab.template.is_none());
        assert!(tab.load_problem.is_none(), "not an error");
        assert_eq!(tab.remembered(), None);
        assert_eq!(
            tab.note.as_deref(),
            Some("Couldn't reopen access-code.rongta.toml — open the template again.")
        );
    }

    #[test]
    fn an_unreadable_remembered_template_is_noted_and_forgotten() {
        let copies = tempfile::tempdir().unwrap();
        let broken = copies.path().join("broken.rongta.toml");
        std::fs::write(&broken, "not a template").unwrap();

        let mut tab = TemplateTab::new(Some(copies.path().into()), Some(broken.clone()));
        tab.reopen(broken);
        assert!(tab.template.is_none());
        assert_eq!(tab.remembered(), None);
        assert_eq!(
            tab.note.as_deref(),
            Some("Couldn't reopen broken.rongta.toml — open the template again.")
        );
    }

    #[test]
    fn a_bad_file_keeps_the_remembered_template() {
        let remembered = PathBuf::from("/kept/access-code.rongta.toml");
        let mut tab = TemplateTab::new(None, Some(remembered.clone()));
        tab.open(PathBuf::from("/nonexistent/template.rongta.toml"));
        assert_eq!(tab.remembered(), Some(remembered.as_path()));
    }

    const BADGE: &str = include_str!("../tests/fixtures/name-badge.rongta.toml");

    /// The problem the preview would show for `name` on the name badge, or
    /// for the example's code when `template` is the access-code one.
    fn real_problem(template: &str, field: &str, value: &str) -> Problem {
        let template = Template::parse(template).unwrap();
        let values = template.normalize(&Values::from([(field.to_owned(), value.to_owned())]));
        let error = template.raster(&values, HEAD_DOTS).unwrap_err();
        render_problem(&error)
    }

    fn unpatterned_example() -> String {
        std::fs::read_to_string(example())
            .unwrap()
            .replace("pattern = \"[A-Z0-9]{6,12}\"\n", "")
    }

    #[test]
    fn a_qr_code_too_long_to_encode_says_so() {
        let problem = real_problem(&unpatterned_example(), "code", &"A".repeat(10_000));
        assert!(
            problem
                .summary
                .contains("The QR content is too long to encode"),
            "{}",
            problem.summary
        );
    }

    #[test]
    fn a_qr_code_too_dense_to_scan_says_so() {
        let problem = real_problem(&unpatterned_example(), "code", &"A".repeat(300));
        assert!(
            problem.summary.contains("too many to scan on this label"),
            "{}",
            problem.summary
        );
    }

    #[test]
    fn text_too_long_beside_the_qr_code_says_so() {
        let problem = real_problem(BADGE, "name", "Bartholomew Montgomery-Fitzwilliam");
        assert!(
            problem
                .summary
                .contains("too long to fit beside the QR code"),
            "{}",
            problem.summary
        );
    }

    #[test]
    fn an_unprintable_character_says_so() {
        let problem = real_problem(BADGE, "name", "\u{1F600}");
        assert!(
            problem
                .summary
                .contains("which the label font cannot print"),
            "{}",
            problem.summary
        );
    }
}
