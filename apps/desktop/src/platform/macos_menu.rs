use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

#[derive(Clone, Copy, Debug)]
pub enum Action {
    ImportAudio,
    SaveProject,
    OpenProject,
    Quit,
    Undo,
    Redo,
    AddTrack,
    ToggleDevicePanel,
    ToggleMixer,
    TogglePianoRoll,
}

pub struct NativeMenu {
    // Keep the root menu alive for as long as the native application menu is used.
    _menu: Menu,
    actions: Vec<(muda::MenuId, Action)>,
}

impl NativeMenu {
    pub fn install() -> Result<Self, String> {
        let mut actions = Vec::new();

        let app_quit = item("app.quit", "Quit Esperia", Action::Quit, &mut actions);
        let app_menu = Submenu::with_items("Esperia", true, &[&app_quit])
            .map_err(|error| error.to_string())?;

        let import = item(
            "file.import",
            "Import Audio…",
            Action::ImportAudio,
            &mut actions,
        );
        let save = item(
            "file.save",
            "Save Project…",
            Action::SaveProject,
            &mut actions,
        );
        let open = item(
            "file.open",
            "Open Project…",
            Action::OpenProject,
            &mut actions,
        );
        let file_quit = item("file.quit", "Quit", Action::Quit, &mut actions);
        let file_menu = Submenu::with_items(
            "File",
            true,
            &[
                &import,
                &save,
                &open,
                &PredefinedMenuItem::separator(),
                &file_quit,
            ],
        )
        .map_err(|error| error.to_string())?;

        let undo = item("edit.undo", "Undo", Action::Undo, &mut actions);
        let redo = item("edit.redo", "Redo", Action::Redo, &mut actions);
        let add_track = item(
            "edit.add_track",
            "Add Track",
            Action::AddTrack,
            &mut actions,
        );
        let edit_menu = Submenu::with_items(
            "Edit",
            true,
            &[&undo, &redo, &PredefinedMenuItem::separator(), &add_track],
        )
        .map_err(|error| error.to_string())?;

        let device = item(
            "view.device",
            "Toggle Device & Tone Panel",
            Action::ToggleDevicePanel,
            &mut actions,
        );
        let mixer = item(
            "view.mixer",
            "Toggle Mixer",
            Action::ToggleMixer,
            &mut actions,
        );
        let piano = item(
            "view.piano_roll",
            "Toggle Piano Roll",
            Action::TogglePianoRoll,
            &mut actions,
        );
        let view_menu = Submenu::with_items("View", true, &[&device, &mixer, &piano])
            .map_err(|error| error.to_string())?;

        let menu = Menu::with_items(&[&app_menu, &file_menu, &edit_menu, &view_menu])
            .map_err(|error| error.to_string())?;
        menu.init_for_nsapp();

        Ok(Self {
            _menu: menu,
            actions,
        })
    }

    pub fn poll_actions(&self) -> impl Iterator<Item = Action> + '_ {
        std::iter::from_fn(|| {
            let event = MenuEvent::receiver().try_recv().ok()?;
            self.actions
                .iter()
                .find_map(|(id, action)| (id == &event.id).then_some(*action))
        })
    }
}

fn item(
    id: &'static str,
    label: &str,
    action: Action,
    actions: &mut Vec<(muda::MenuId, Action)>,
) -> MenuItem {
    let item = MenuItem::with_id(id, label, true, None);
    actions.push((item.id().clone(), action));
    item
}
