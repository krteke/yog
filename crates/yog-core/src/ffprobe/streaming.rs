use crate::ffprobe::types::MediaStream;

use super::types::ProbeError;
use serde::de::{DeserializeOwned, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use std::{fmt, io::Read, marker::PhantomData};

/// Visit the JSON array directly; no intermediate Value or Vec of records.
pub(super) fn records<T: DeserializeOwned>(
    reader: impl Read,
    key: &str,
    consume: &mut impl FnMut(T),
) -> Result<Option<ProbeError>, serde_json::Error> {
    let mut decoder = serde_json::Deserializer::from_reader(reader);
    let error = serde::de::Deserializer::deserialize_map(
        &mut decoder,
        Root {
            key,
            consume,
            marker: PhantomData,
        },
    )?;
    decoder.end()?;
    Ok(error)
}

struct Root<'a, T, F> {
    key: &'a str,
    consume: &'a mut F,
    marker: PhantomData<T>,
}
impl<'de, T: DeserializeOwned, F: FnMut(T)> Visitor<'de> for Root<'_, T, F> {
    type Value = Option<ProbeError>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("ffprobe JSON object")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut error = None;
        while let Some(key) = map.next_key::<String>()? {
            if key == self.key {
                map.next_value_seed(Records {
                    consume: &mut *self.consume,
                    marker: PhantomData,
                })?;
            } else if key == "error" {
                error = map.next_value()?;
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(error)
    }
}

struct Records<'a, T, F> {
    consume: &'a mut F,
    marker: PhantomData<T>,
}
impl<'de, T: DeserializeOwned, F: FnMut(T)> DeserializeSeed<'de> for Records<'_, T, F> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_seq(self)
    }
}
impl<'de, T: DeserializeOwned, F: FnMut(T)> Visitor<'de> for Records<'_, T, F> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("ffprobe record array")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        while let Some(record) = seq.next_element::<T>()? {
            (self.consume)(record);
        }
        Ok(())
    }
}

impl MediaStream {
    pub fn kind(&self) -> &str {
        if self.codec_type.as_deref() == Some("video") && !self.is_regular_video() {
            "cover"
        } else {
            self.codec_type.as_deref().unwrap_or("unknown")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffprobe::types::Packet;
    use std::{
        cell::Cell,
        io::{self, Cursor},
        rc::Rc,
    };

    struct IncrementalInput {
        consumed: Rc<Cell<usize>>,
        emitted: usize,
        current: Cursor<Vec<u8>>,
        ended: bool,
    }
    impl Read for IncrementalInput {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = self.current.read(buffer)?;
            if count != 0 || self.ended {
                return Ok(count);
            }
            // A collecting decoder would request another record before exposing
            // the previous one, failing here long before the large input ends.
            assert_eq!(self.consumed.get(), self.emitted);
            if self.emitted == 50_000 {
                self.current = Cursor::new(b"]}".to_vec());
                self.ended = true;
            } else {
                let separator = if self.emitted == 0 { "" } else { "," };
                self.current = Cursor::new(
                    format!(
                        "{separator}{{\"stream_index\":0,\"size\":\"123\",\"unused\":\"{}\"}}",
                        "x".repeat(400)
                    )
                    .into_bytes(),
                );
                self.emitted += 1;
            }
            self.current.read(buffer)
        }
    }

    #[test]
    fn more_than_sixteen_mib_is_consumed_record_by_record() {
        let count = Rc::new(Cell::new(0));
        let reader = IncrementalInput {
            consumed: count.clone(),
            emitted: 0,
            current: Cursor::new(b"{\"packets\":[".to_vec()),
            ended: false,
        };
        let error = records(reader, "packets", &mut |packet: Packet| {
            assert_eq!(packet.size.as_deref(), Some("123"));
            count.set(count.get() + 1);
        })
        .unwrap();
        assert!(error.is_none());
        assert_eq!(count.get(), 50_000);
    }
}
