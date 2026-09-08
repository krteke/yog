use std::{ffi::OsString, path::Path};

pub(super) enum Arg<'a> {
    /// -v error
    ErrorsOnly,
    /// -of json
    Json,
    /// -show_error
    ShowError,
    /// -show_format
    ShowFormat,
    /// -show_streams
    ShowStreams,
    /// -show_chapters
    ShowChapters,
    /// -show_programs
    ShowPrograms,
    /// -show_frames
    ShowFrames,
    /// -show_packets
    ShowPackets,
    /// -select_stream
    SelectStream(usize),
    /// -read_intervals
    ReadIntervals(&'a str),
    /// -show_entries
    ShowEntries(Entries),
    Input(&'a Path),
}

pub(super) enum Entries {
    Frame,
    Packet,
}

impl From<Entries> for OsString {
    fn from(entries: Entries) -> Self {
        match entries {
            Entries::Frame => "frame=stream_index,media_type,pts_time,best_effort_timestamp_time,duration_time,pix_fmt,color_range,color_space,color_transfer,color_primaries,chroma_location,interlaced_frame,top_field_first:frame_side_data",
            Entries::Packet => "packet=stream_index,pts_time,dts_time,duration_time,size,flags",
        }.into()
    }
}

pub(super) trait ArgsExt {
    fn add(&mut self, arg: Arg<'_>);
}

impl ArgsExt for Vec<OsString> {
    fn add(&mut self, arg: Arg<'_>) {
        let (flag, value): (&str, Option<OsString>) = match arg {
            Arg::ErrorsOnly => ("-v", Some("error".into())),
            Arg::Json => ("-of", Some("json".into())),
            Arg::ShowError => ("-show_error", None),
            Arg::ShowFormat => ("-show_format", None),
            Arg::ShowStreams => ("-show_streams", None),
            Arg::ShowChapters => ("-show_chapters", None),
            Arg::ShowPrograms => ("-show_programs", None),
            Arg::ShowFrames => ("-show_frames", None),
            Arg::ShowPackets => ("-show_packets", None),
            Arg::SelectStream(index) => ("-select_streams", Some(index.to_string().into())),
            Arg::ReadIntervals(intervals) => ("-read_intervals", Some(intervals.into())),
            Arg::ShowEntries(entries) => ("-show_entries", Some(entries.into())),
            Arg::Input(path) => ("--", Some(path.as_os_str().to_owned())),
        };
        self.push(flag.into());
        self.extend(value);
    }
}

impl<'a> Extend<Arg<'a>> for Vec<OsString> {
    fn extend<T: IntoIterator<Item = Arg<'a>>>(&mut self, args: T) {
        for arg in args {
            self.add(arg);
        }
    }
}
