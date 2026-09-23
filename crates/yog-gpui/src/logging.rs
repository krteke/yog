use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};
use tokio::sync::mpsc::UnboundedSender;

use crate::workspace::messages::{MessageLevel, MessageSource, NewMessage};

static LOGGER: GuiLogger = GuiLogger {
    sink: Mutex::new(None),
};

struct GuiLogger {
    sink: Mutex<Option<UnboundedSender<NewMessage>>>,
}

pub fn init() {
    log::set_logger(&LOGGER).expect("GUI logger must be installed once");
    log::set_max_level(LevelFilter::Debug);
}

pub struct Capture;

pub fn capture(sender: UnboundedSender<NewMessage>) -> Capture {
    *LOGGER.sink.lock().unwrap() = Some(sender);
    Capture
}

impl Drop for Capture {
    fn drop(&mut self) {
        *LOGGER.sink.lock().unwrap() = None;
    }
}

impl Log for GuiLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= Level::Debug
            && (metadata.target().starts_with("yog_core")
                || metadata.target().starts_with("yog_runtime")
                || metadata.target().starts_with("yog::"))
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let source = match record.target() {
            "yog::ffmpeg" => MessageSource::Ffmpeg,
            "yog::command" => MessageSource::Command,
            _ => MessageSource::Program,
        };
        let level = match record.level() {
            Level::Error => MessageLevel::Error,
            Level::Warn => MessageLevel::Warning,
            Level::Info => MessageLevel::Info,
            Level::Debug | Level::Trace => MessageLevel::Debug,
        };
        let message = NewMessage {
            source,
            level,
            text: record.args().to_string(),
        };
        if let Some(sender) = self.sink.lock().unwrap().as_ref() {
            _ = sender.send(message);
        }
    }

    fn flush(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_command_and_ffmpeg_records_to_separate_sources() {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let _capture = capture(sender);

        LOGGER.log(
            &Record::builder()
                .level(Level::Debug)
                .target("yog::command")
                .args(format_args!("executing command: ffmpeg -i input"))
                .build(),
        );
        LOGGER.log(
            &Record::builder()
                .level(Level::Debug)
                .target("yog::ffmpeg")
                .args(format_args!("frame=10\n"))
                .build(),
        );

        let command = receiver.try_recv().unwrap();
        assert_eq!(command.source, MessageSource::Command);
        assert_eq!(command.text, "executing command: ffmpeg -i input");

        let ffmpeg = receiver.try_recv().unwrap();
        assert_eq!(ffmpeg.source, MessageSource::Ffmpeg);
        assert_eq!(ffmpeg.text, "frame=10\n");
    }
}
