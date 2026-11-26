use tauri::WebviewUrl;
use tauri::WebviewWindowBuilder;

pub fn create_count_metronome_button(app: &tauri::AppHandle) {
    let html = r##"
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <title>Count-in & Metronome Button</title>
  <style>
    body {
      background: #9e9e9e;
      display: flex;
      justify-content: center;
      align-items: center;
      height: 100vh;
      margin: 0;
    }

    .pill-container {
      position: relative;
      display: inline-flex;
      gap: 2px;
      height: 40px;
      background-color: transparent;
      border-radius: 14px;
      corner-shape: squircle;
      user-select: none;
      transition: background-color 0.16s ease;
    }

    .pill-container:hover {
      background-color: #8c8c8c;
    }

    .slot-btn {
      position: relative;
      width: 46px;
      height: 40px;
      border: none;
      background: transparent;
      border-radius: 14px;
      corner-shape: squircle;
      padding: 0;
      margin: 0;
      cursor: pointer;
      display: flex;
      align-items: center;
      justify-content: center;
      color: #515151;
      outline: none;
      transform: translateY(0) scale(1);
      box-shadow: inset 0 0 0 rgba(0, 0, 0, 0);
      transition:
        background-color 0.16s ease,
        color 0.16s ease,
        box-shadow 0.18s ease,
        transform 0.34s cubic-bezier(0.22, 1.65, 0.36, 1);
    }

    .slot-btn:active {
      transform: scale(0.94);
      box-shadow:
        inset 0 3px 6px rgba(0, 0, 0, 0.28),
        inset 0 1px 2px rgba(0, 0, 0, 0.22);
      transition:
        transform 0.06s ease-out,
        box-shadow 0.06s ease-out;
    }

    .slot-btn.active {
      background: linear-gradient(180deg, #8b33b8 0%, #73259b 100%);
      color: #f5e8ff;
      box-shadow:
        inset 0 1px 0.5px rgba(255, 255, 255, 0.32),
        inset 0 -1.5px 2px rgba(32, 7, 46, 0.35),
        0 3px 8px rgba(78, 22, 108, 0.32);
    }

    .slot-btn.active:active {
      background: linear-gradient(180deg, #6e2394 0%, #7d29a7 100%);
      transform: scale(0.94);
      box-shadow:
        inset 0 3px 6px rgba(25, 5, 36, 0.48),
        inset 0 1px 2px rgba(0, 0, 0, 0.3);
    }

    .slot-btn svg {
      width: 46px;
      height: 40px;
      display: block;
      pointer-events: none;
      filter: drop-shadow(0 1px 0 rgba(255, 255, 255, 0.12));
      transition:
        transform 0.36s cubic-bezier(0.22, 1.75, 0.36, 1),
        filter 0.16s ease;
    }

    .slot-btn.active svg {
      filter: drop-shadow(0 1px 1.5px rgba(25, 5, 36, 0.4));
    }

    .slot-btn:active svg {
      transform: scale(0.92);
      transition: transform 0.06s ease-out;
    }
  </style>
</head>
<body>

  <div class="pill-container">

    <button class="slot-btn" id="countinBtn"
            type="button" title="Count-in">
      <svg width="46" height="40" viewBox="0 0 46 40" fill="none">
        <path d="M11.5 19.5C15.5 13.5 23.5 10 32.5 9.5"
          stroke="currentColor" stroke-opacity="0.45"
          stroke-width="1.6" stroke-linecap="round"
          stroke-dasharray="2 2.5"/>

        <rect x="10.5" y="23" width="3.8" height="6"
          rx="1.9" fill="currentColor" fill-opacity="0.5"/>

        <rect x="17.5" y="19.5" width="3.8" height="9.5"
          rx="1.9" fill="currentColor" fill-opacity="0.7"/>

        <rect x="24.5" y="16" width="3.8" height="13"
          rx="1.9" fill="currentColor" fill-opacity="0.88"/>

        <rect id="peakBar" x="31.5" y="12"
          width="4.2" height="17" rx="2.1"
          fill="currentColor"/>
      </svg>
    </button>

    <button class="slot-btn" id="metronomeBtn"
            type="button" title="Metronome">
      <svg width="46" height="40" viewBox="0 0 46 40" fill="none">
        <circle cx="23" cy="10.2" r="2.1" fill="currentColor"/>
        <rect x="20.9" y="14.2" width="4.2" height="14.8"
          rx="2.1" fill="currentColor"/>

        <path d="M16.2 17.1C14.4 19.5 14.4 23.7 16.2 26.1"
          stroke="currentColor" stroke-opacity="0.85"
          stroke-width="2.1" stroke-linecap="round"/>

        <path d="M29.8 17.1C31.6 19.5 31.6 23.7 29.8 26.1"
          stroke="currentColor" stroke-opacity="0.85"
          stroke-width="2.1" stroke-linecap="round"/>

        <path d="M12.0 14.6C9.4 18.4 9.4 24.8 12.0 28.6"
          stroke="currentColor" stroke-opacity="0.45"
          stroke-width="1.7" stroke-linecap="round"
          stroke-dasharray="2 2.5"/>

        <path d="M34.0 14.6C36.6 18.4 36.6 24.8 34.0 28.6"
          stroke="currentColor" stroke-opacity="0.45"
          stroke-width="1.7" stroke-linecap="round"
          stroke-dasharray="2 2.5"/>
      </svg>
    </button>

  </div>

  <script>
    const countinBtn = document.getElementById('countinBtn');
    const metronomeBtn = document.getElementById('metronomeBtn');
    const peakBar = document.getElementById('peakBar');

    countinBtn.addEventListener('click', () => {
      const isActive = countinBtn.classList.toggle('active');
      peakBar.setAttribute(
        'fill',
        isActive ? '#ffffff' : 'currentColor'
      );
    });

    metronomeBtn.addEventListener('click', () => {
      metronomeBtn.classList.toggle('active');
    });
  </script>

</body>
</html>
"##;

    let url = format!("data:text/html;charset=utf-8,{}", urlencoding::encode(html));

    WebviewWindowBuilder::new(
        app,
        "count-metronome-button",
        WebviewUrl::External(url.parse().expect("Invalid HTML data URL")),
    )
    .title("Count-in & Metronome")
    .inner_size(160.0, 100.0)
    .resizable(false)
    .decorations(false)
    .transparent(false)
    .build()
    .expect("Failed to create Count-in & Metronome window");
}

/// Build the Tauri app and spawn `create_count_metronome_button` during `.setup`.
#[allow(dead_code)]
pub fn build_tauri_app() -> tauri::Result<tauri::App> {
    tauri::Builder::default()
        .setup(|app| {
            create_count_metronome_button(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
}

/// Host toolbar widget handle used by `EsperiaApp` in `main.rs`.
#[derive(Default)]
pub struct CountMetronomeButton {
    pub countin_active: bool,
    pub metronome_active: bool,
}

impl CountMetronomeButton {
    pub fn ui(
        &mut self,
        ui: &mut eframe::egui::Ui,
        _background: eframe::egui::Color32,
    ) -> eframe::egui::Response {
        let (rect, mut response) = ui.allocate_exact_size(
            eframe::egui::Vec2::new(94.0, 40.0),
            eframe::egui::Sense::hover(),
        );
        let left = eframe::egui::Rect::from_min_size(rect.min, eframe::egui::Vec2::new(46.0, 40.0));
        let right = eframe::egui::Rect::from_min_size(
            eframe::egui::pos2(rect.min.x + 48.0, rect.min.y),
            eframe::egui::Vec2::new(46.0, 40.0),
        );

        let r1 = ui.put(
            left,
            eframe::egui::Button::new("1234")
                .fill(if self.countin_active {
                    eframe::egui::Color32::from_rgb(127, 44, 170)
                } else {
                    eframe::egui::Color32::TRANSPARENT
                })
                .corner_radius(14.0),
        );
        if r1.clicked() {
            self.countin_active = !self.countin_active;
            response.mark_changed();
        }

        let r2 = ui.put(
            right,
            eframe::egui::Button::new("△")
                .fill(if self.metronome_active {
                    eframe::egui::Color32::from_rgb(127, 44, 170)
                } else {
                    eframe::egui::Color32::TRANSPARENT
                })
                .corner_radius(14.0),
        );
        if r2.clicked() {
            self.metronome_active = !self.metronome_active;
            response.mark_changed();
        }

        response
    }
}
