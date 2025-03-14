use rosetta::*;

use anyhow::anyhow;
use config::Config;
use eframe::egui::{Button, Color32, TextBuffer, TextEdit};
use eframe::{egui, Frame};
use log::LevelFilter;
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender};
use chrono::Local;
use tokio::task::JoinHandle;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() {
    // TODO: Log window and/file
    // TODO: Last used file
    env_logger::Builder::new()
        .filter(None, LevelFilter::Debug)
        .format(|buf, record| {
            use std::io::Write;

            let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
            let level = record.level();
            let target = record.target();

            let thread = std::thread::current();
            writeln!(
                buf,
                "{} {: <5} {} - {} [{}]",
                timestamp,
                level,
                target,
                record.args(),
                thread.name().unwrap_or("<unnamed>")
            )
        })
        .init();

    // Log all errors happening via tracing crate (used by e.g. OpenAI)
    tracing::subscriber::set_global_default(
        tracing_subscriber::FmtSubscriber::builder()
            .with_max_level(tracing::Level::ERROR)
            .with_writer(std::io::stderr)
            .finish(),
    ).expect("setting default subscriber failed");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 500.0]),
        centered: true,
        ..Default::default()
    };

    let settings = Config::builder()
        .add_source(config::File::with_name("rosetta-settings"))
        .build();

    let (tx, rx) = std::sync::mpsc::channel();
    eframe::run_native(
        &format!("Rosetta v{VERSION}"),
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_zoom_factor(2.0);

            Ok(Box::new(TranslationGui {
                settings,
                input_path: None,
                output_path: "".to_owned(),
                cfg: TranslationConfig::default(),
                tx,
                rx,
                status: None,
                translation_thread: None,
            }))
        }),
    )
    .expect("eframe/egui run failed");
}

#[derive(Debug)]
struct TranslationGui {
    settings: Result<Config, config::ConfigError>,
    input_path: Option<String>,
    output_path: String,
    cfg: TranslationConfig,
    tx: Sender<TranslationStatus>,
    rx: Receiver<TranslationStatus>,
    status: Option<TranslationStatus>,
    translation_thread: Option<JoinHandle<()>>,
}

impl eframe::App for TranslationGui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(format!("Rosetta v{VERSION}"));

            if let Err(ref err) = self.settings {
                self.status = Some(TranslationStatus::Error(TranslationError::OtherError(
                    anyhow!("{err}"),
                )))
            }

            while let Ok(status) = self.rx.try_recv() {
                match status {
                    TranslationStatus::Success | TranslationStatus::Error(_) => {
                        self.translation_thread = None;
                    }
                    _ => {}
                }
                self.status = Some(status);
            }

            ui.horizontal(|ui| {
                let btn = ui
                    .button("Select input file")
                    .on_hover_text("Browse for input file");

                if let Some(mut input_path) = self.input_path.as_deref() {
                    let text_edit =
                        TextEdit::singleline(&mut input_path).desired_width(f32::INFINITY);
                    ui.add(text_edit).labelled_by(btn.id);
                }

                if btn.clicked() {
                    let fd = rfd::FileDialog::new();

                    let fd = match (&self.input_path, &self.settings) {
                        (Some(prev_input_path), _) => {
                            let prev_input_path = Path::new(prev_input_path);
                            fd.set_directory(prev_input_path.parent().expect("parent"))
                        }
                        // (_, Ok(settings)) => {
                        //     let last_input_file = settings
                        //         .get_string("last_input_file")
                        //         .expect("last_input_file config setting");
                        //     if last_input_file.is_empty() {
                        //         fd
                        //     } else {
                        //         let last_input_file = Path::new(&last_input_file);
                        //         fd.set_directory(last_input_file.parent().expect("parent"))
                        //     }
                        // }
                        _ => fd,
                    };

                    if let Some(path) = fd.pick_file() {
                        self.input_path = Some(path.display().to_string());
                        let model = self.settings.as_ref().map(|s| s.get_string("openai.model")).into_iter().flatten().next();
                        let new_file_name = "".to_owned()
                            + path.file_stem().unwrap().to_string_lossy().as_str()
                            + model.map_or("_translated.".to_owned(), |m| format!("_translated_{m}.")).as_str()
                            + path.extension().unwrap().to_string_lossy().as_str();
                        self.output_path = path
                            .parent()
                            .unwrap()
                            .join(new_file_name)
                            .display()
                            .to_string();

                        // if let Ok(settings) = &mut self.settings {
                        //     settings
                        //         .set("last_input_file", path.display().to_string())
                        //         .expect("last_input_file config setting");
                        // }
                    }
                }
            });

            ui.horizontal(|ui| {
                let label = ui
                    .label("Output file:")
                    .on_hover_text("Browse for output file");

                let mut text_edit = self.output_path.as_str();
                let text_edit = TextEdit::singleline(&mut text_edit).desired_width(f32::INFINITY);
                ui.add(text_edit).labelled_by(label.id);
            });

            ui.horizontal(|ui| {
                let label = ui.label("Source language");
                ui.text_edit_singleline(&mut self.cfg.src_lang)
                    .labelled_by(label.id);
            });

            ui.horizontal(|ui| {
                let label = ui.label("Destination language");
                ui.text_edit_singleline(&mut self.cfg.dst_lang)
                    .labelled_by(label.id);
            });

            ui.horizontal(|ui| {
                let label = ui.label("Subject/title");
                let text_edit =
                    TextEdit::singleline(&mut self.cfg.subject).desired_width(f32::INFINITY);
                ui.add(text_edit).labelled_by(label.id);
            });

            ui.horizontal(|ui| {
                let label = ui.label("Tone");
                ui.text_edit_singleline(&mut self.cfg.tone)
                    .labelled_by(label.id);
            });

            ui.horizontal(|ui| {
                let text_edit = TextEdit::multiline(&mut self.cfg.additional_instructions)
                    .desired_width(f32::INFINITY)
                    .hint_text("Additional instructions");
                ui.add(text_edit)
            });

            ui.horizontal(|ui| {
                let btn = ui
                    .add_enabled(
                        self.input_path.is_some()
                            && self.translation_thread.is_none()
                            && self.settings.is_ok(),
                        Button::new("Translate"),
                    )
                    .on_hover_text("Translate the input file");

                let (status_text, status_text_color) = match self.status.as_ref() {
                    Some(TranslationStatus::Started) => {
                        ("Starting translation...".to_owned(), None)
                    }
                    Some(TranslationStatus::Progress(ref progress)) => (
                        format!(
                            "{}/{} sections translated",
                            progress.processed_sections, progress.total_sections
                        ),
                        None,
                    ),
                    Some(TranslationStatus::Success) => {
                        ("Done!".to_owned(), Some(Color32::DARK_GREEN))
                    }
                    Some(TranslationStatus::Error(ref error)) => {
                        (format!("{}", error), Some(Color32::RED))
                    }
                    None => ("".to_owned(), None),
                };

                let mut status_text = status_text.as_str();
                ui.add(
                    TextEdit::singleline(&mut status_text)
                        .desired_width(f32::INFINITY)
                        .text_color_opt(status_text_color),
                );

                if btn.clicked() {
                    self.status = None;

                    let settings = self.settings.as_ref().unwrap().clone();
                    let input_path = self.input_path.as_ref().unwrap().clone();
                    let output_path = self.output_path.clone();
                    let cfg = self.cfg.clone();
                    let tx = self.tx.clone();

                    self.translation_thread = Some(tokio::spawn(async move {
                        tx.send(TranslationStatus::Started).unwrap();

                        let send_progress = SendProgressThroughChannel { tx: tx.clone() };
                        let translation_res = tokio::spawn(async move {
                            translate(
                                settings,
                                Path::new(&input_path),
                                Path::new(&output_path),
                                cfg,
                                send_progress,
                            )
                            .await
                        })
                        .await;
                        match translation_res {
                            Ok(Ok(())) => {
                                tx.send(TranslationStatus::Success).unwrap();
                            }
                            Ok(Err(failure)) => {
                                tx.send(TranslationStatus::Error(failure)).unwrap();
                            }
                            Err(_) => {
                                tx.send(TranslationStatus::Error(TranslationError::OtherError(
                                    anyhow!("Crash!"),
                                )))
                                .unwrap();
                            }
                        }
                    }));
                };
            });
        });
    }
}

struct SendProgressThroughChannel {
    tx: Sender<TranslationStatus>,
}

impl SendProgress for SendProgressThroughChannel {
    fn send_progress(&self, progress: Progress) {
        self.tx
            .send(TranslationStatus::Progress(progress))
            .expect("send");
    }
}
