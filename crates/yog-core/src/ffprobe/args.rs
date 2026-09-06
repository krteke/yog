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

impl Arg<'_> {
    /// An option expands to one or two argv elements, never a shell fragment.
    pub fn append_to(self, args: &mut Vec<OsString>) {
        let (flag, value) = match self {
            Self::ErrorsOnly => ("-v", Some("error".into())),
            Self::Json => ("-of", Some("json".into())),
            Self::ShowError => ("-show_error", None),
            Self::ShowFormat => ("-show_format", None),
            Self::ShowStreams => ("-show_streams", None),
            Self::ShowChapters => ("-show_chapters", None),
            Self::ShowPrograms => ("-show_programs", None),
            Self::ShowFrames => ("-show_frames", None),
            Self::ShowPackets => ("-show_packets", None),
            Self::SelectStream(index) => ("-select_streams", Some(index.to_string().into())),
            Self::ReadIntervals(intervals) => ("-read_intervals", Some(intervals.into())),
            Self::ShowEntries(entries) => ("-show_entries", Some(entries.into())),
            Self::Input(path) => ("--", Some(path.as_os_str().to_owned())),
        };
        args.push(flag.into());
        args.extend(value);
    }
}
