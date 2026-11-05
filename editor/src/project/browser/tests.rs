use super::*;
use anyhow::{Result, ensure};
use egui::{Event, PointerButton, Pos2, Rect, Vec2};
use std::{
    fs,
    time::{Duration, Instant},
};

struct Ui {
    browser: ProjectBrowser,
    reader: DirectoryReader,
    ctx: egui::Context,
    action: Option<DocumentAction>,
}
impl Ui {
    fn new(create: bool, current: &Path) -> Self {
        let mut browser = ProjectBrowser::default();
        browser.open(create, current);
        let ctx = egui::Context::default();
        ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            ctx.style_mut_of(theme, |style| style.animation_time = 0.);
        }
        Self {
            browser,
            reader: Default::default(),
            ctx,
            action: None,
        }
    }
    fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        self.reader.poll();
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000., 900.))),
                events,
                ..Default::default()
            },
            |_| {
                let action = self.browser.show(&self.ctx, &mut self.reader);
                if action.is_some() {
                    self.action = action;
                }
            },
        );
        output.textures_delta.clear();
        output
    }
    fn click(&mut self, position: Pos2) -> egui::FullOutput {
        let button = |pressed| Event::PointerButton {
            pos: position,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        self.frame(vec![Event::PointerMoved(position), button(true)]);
        self.frame(vec![button(false)])
    }
    fn press(&mut self, label: &str) -> egui::FullOutput {
        self.frame(vec![]);
        let output = self.frame(vec![]);
        self.click(point(&output, label))
    }
    fn wait(&mut self) -> Result<egui::FullOutput> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let output = self.frame(vec![]);
            if let Some(picker) = &self.browser.picker
                && self.reader.listing(picker.request).is_some()
            {
                return Ok(output);
            }
            ensure!(Instant::now() < deadline, "directory UI did not finish");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
fn texts(output: &egui::FullOutput) -> Vec<(String, Pos2)> {
    fn collect(shape: &egui::epaint::Shape, out: &mut Vec<(String, Pos2)>) {
        match shape {
            egui::epaint::Shape::Text(text) => out.push((
                text.galley.text().into(),
                text.pos + text.galley.rect.center().to_vec2(),
            )),
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, out);
                }
            }
            _ => {}
        }
    }
    let mut out = vec![];
    for shape in &output.shapes {
        collect(&shape.shape, &mut out);
    }
    out
}
fn point(output: &egui::FullOutput, label: &str) -> Pos2 {
    texts(output)
        .into_iter()
        .rev()
        .find(|(text, _)| text == label)
        .unwrap_or_else(|| panic!("missing {label}: {:?}", texts(output)))
        .1
}
fn escape() -> Event {
    Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }
}

#[test]
fn project_and_nested_native_author_selection_only_emit_an_action_after_final_confirmation()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let current = temp.path().join("current");
    let target = temp.path().join(" target ");
    fs::create_dir_all(current.join("assets"))?;
    fs::create_dir_all(target.join("assets/ nested "))?;
    let target = target.canonicalize()?;
    let author = PathBuf::from(" nested / source.prefab");
    fs::write(target.join("assets").join(&author), [])?;
    let mut ui = Ui::new(false, &current);
    ui.press("浏览目录");
    ui.wait()?;
    ui.press("上一级");
    ui.wait()?;
    ui.press("目录   target ");
    ui.press("选择目录");
    assert!(ui.action.is_none());
    assert_eq!(ui.browser.root_path(), Some(target.clone()));
    ui.press("浏览作者文件");
    ui.wait()?;
    ui.press("目录   nested ");
    ui.press("进入目录");
    ui.wait()?;
    ui.press("作者文件   source.prefab");
    ui.press("选择作者文件");
    assert!(ui.action.is_none());
    assert_eq!(ui.browser.scene_path(), Some(author.clone()));
    ui.press("打开");
    assert_eq!(
        ui.action,
        Some(DocumentAction::OpenProject {
            root: target,
            scene: author
        })
    );
    assert!(!ui.browser.is_open());
    Ok(())
}

#[test]
fn parent_selection_preserves_the_new_name_and_escape_only_cancels_the_active_page() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let current = temp.path().join("current");
    let parent = temp.path().join(" parent ");
    fs::create_dir_all(current.join("assets"))?;
    fs::create_dir(&parent)?;
    let parent = parent.canonicalize()?;
    let mut ui = Ui::new(true, &current);
    ui.press("选择父目录");
    ui.wait()?;
    ui.press("目录   parent ");
    ui.press("选择目录");
    let expected = parent.join("NewProject");
    assert_eq!(ui.browser.root_path(), Some(expected.clone()));
    assert!(!expected.exists());
    ui.press("选择父目录");
    ui.wait()?;
    ui.frame(vec![escape()]);
    assert!(ui.browser.is_open() && ui.browser.picker.is_none());
    ui.frame(vec![escape()]);
    assert!(!ui.browser.is_open());
    assert!(ui.action.is_none());
    assert!(!expected.exists());
    ui.browser.open(true, &current);
    ui.browser.picked_root = Some(expected.clone());
    ui.browser.root = expected.display().to_string();
    ui.press("创建并打开");
    assert_eq!(ui.action, Some(DocumentAction::NewProject(expected)));
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn clicked_non_utf8_author_and_project_paths_never_round_trip_through_display_text() -> Result<()> {
    use std::os::unix::ffi::OsStringExt;
    let temp = tempfile::tempdir()?;
    let project = temp
        .path()
        .join(std::ffi::OsString::from_vec(b" project-\xff ".to_vec()));
    fs::create_dir_all(project.join("assets"))?;
    let author = PathBuf::from(std::ffi::OsString::from_vec(
        b" source-\xfe.prefab".to_vec(),
    ));
    fs::write(project.join("assets").join(&author), [])?;
    let mut ui = Ui::new(false, &project);
    ui.press("浏览作者文件");
    let output = ui.wait()?;
    let label = format!("作者文件  {}", author.display());
    ui.click(point(&output, &label));
    ui.press("选择作者文件");
    ui.press("打开");
    assert_eq!(
        ui.action,
        Some(DocumentAction::OpenProject {
            root: project,
            scene: author
        })
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn selected_native_paths_survive_lossy_labels_without_filesystem_encoding_assumptions() {
    use std::os::unix::ffi::OsStringExt;
    let project = PathBuf::from(std::ffi::OsString::from_vec(b"/ project-\xff ".to_vec()));
    let author = PathBuf::from(std::ffi::OsString::from_vec(
        b" source-\xfe.prefab".to_vec(),
    ));
    let mut ui = Ui::new(false, &project);
    // A picker returns native paths; the UI keeps these separately from its lossy text labels.
    ui.browser.picked_scene = Some(author.clone());
    ui.browser.scene = author.display().to_string();
    ui.press("打开");
    assert_eq!(
        ui.action,
        Some(DocumentAction::OpenProject {
            root: project,
            scene: author
        })
    );
}
