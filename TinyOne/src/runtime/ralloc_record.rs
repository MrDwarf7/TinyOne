//! A fixed-shape named-field record backed by [`RallocBytes`] — used for
//! `HeapData::Struct` and `HeapData::Enum` (and, structurally identically,
//! `Record`).
//!
//! Unlike [`crate::runtime::ralloc_vec::RallocVec`], this never resizes:
//! `runtime_set_field` only ever overwrites an existing field's value slot,
//! never adds or removes a field, so the whole record (field names, an
//! optional enum tag/variant name, and the value slots) is sized exactly
//! once at construction.
//!
//! Byte layout:
//! ```text
//! [0..4)   tag: u32 LE                 (Enum only; 0 for Struct/Record)
//! [4..6)   variant_len: u16 LE         (Enum only; 0 for Struct/Record)
//! [6..8)   field_count: u16 LE
//! [8..8+2*field_count)  name_len[i]: u16 LE, one per field
//! ..variant name bytes.. (variant_len bytes)
//! ..field name blob..    (concatenated UTF-8 field names, in order)
//! ..value slots..        (field_count * value_codec::ENCODED_VALUE_BYTES)
//! ```
//! Field name offsets are never stored explicitly — since names are laid
//! out contiguously in construction order, an offset is always the prefix
//! sum of the preceding lengths. Field counts are always small (real
//! structs/enums have a handful of fields), so the O(field_count) work this
//! implies for lookups is negligible.

use crate::runtime::value_codec::{self, ENCODED_VALUE_BYTES};
use crate::tiny_allocator::RallocBytes;
use crate::{Result, TinyOneError, Value};

const OFF_TAG: usize = 0;
const OFF_VARIANT_LEN: usize = 4;
const OFF_FIELD_COUNT: usize = 6;
const OFF_NAME_LENS: usize = 8;

pub(crate) struct RallocRecord {
    bytes: RallocBytes,
}

impl RallocRecord {
    /// Builds a record from `fields` (in order) plus optional enum metadata.
    /// Pass `tag: 0, variant: ""` for a plain `Struct`/`Record`.
    pub(crate) fn new(tag: u32, variant: &str, fields: &[(String, Value)]) -> Result<Self> {
        let field_count = fields.len();
        if field_count > u16::MAX as usize {
            return Err(TinyOneError::runtime("too many fields for a record"));
        }
        let variant_bytes = variant.as_bytes();
        if variant_bytes.len() > u16::MAX as usize {
            return Err(TinyOneError::runtime("enum variant name too long"));
        }

        let mut name_lens = Vec::with_capacity(field_count);
        let mut total_name_bytes = 0usize;
        for (name, _) in fields {
            let len = name.as_bytes().len();
            if len > u16::MAX as usize {
                return Err(TinyOneError::runtime(format!(
                    "field name {name:?} too long"
                )));
            }
            name_lens.push(len as u16);
            total_name_bytes += len;
        }

        let header_len = OFF_NAME_LENS + field_count * 2;
        let total_len =
            header_len + variant_bytes.len() + total_name_bytes + field_count * ENCODED_VALUE_BYTES;

        let mut buf = vec![0u8; total_len];
        buf[OFF_TAG..OFF_TAG + 4].copy_from_slice(&tag.to_le_bytes());
        buf[OFF_VARIANT_LEN..OFF_VARIANT_LEN + 2]
            .copy_from_slice(&(variant_bytes.len() as u16).to_le_bytes());
        buf[OFF_FIELD_COUNT..OFF_FIELD_COUNT + 2]
            .copy_from_slice(&(field_count as u16).to_le_bytes());
        for (i, len) in name_lens.iter().enumerate() {
            let off = OFF_NAME_LENS + i * 2;
            buf[off..off + 2].copy_from_slice(&len.to_le_bytes());
        }

        let mut cursor = header_len;
        buf[cursor..cursor + variant_bytes.len()].copy_from_slice(variant_bytes);
        cursor += variant_bytes.len();
        for (name, _) in fields {
            let name_bytes = name.as_bytes();
            buf[cursor..cursor + name_bytes.len()].copy_from_slice(name_bytes);
            cursor += name_bytes.len();
        }
        for (_, value) in fields {
            let encoded = value_codec::encode_value(value)?;
            buf[cursor..cursor + ENCODED_VALUE_BYTES].copy_from_slice(&encoded);
            cursor += ENCODED_VALUE_BYTES;
        }
        debug_assert_eq!(cursor, total_len);

        let bytes = RallocBytes::from_slice(&buf)
            .map_err(|e| TinyOneError::runtime(format!("Failed to allocate record: {e}")))?;
        Ok(Self { bytes })
    }

    fn field_count(&self) -> usize {
        u16::from_le_bytes(
            self.bytes.as_slice()[OFF_FIELD_COUNT..OFF_FIELD_COUNT + 2]
                .try_into()
                .unwrap(),
        ) as usize
    }

    fn variant_len(&self) -> usize {
        u16::from_le_bytes(
            self.bytes.as_slice()[OFF_VARIANT_LEN..OFF_VARIANT_LEN + 2]
                .try_into()
                .unwrap(),
        ) as usize
    }

    fn name_len(&self, index: usize) -> usize {
        let off = OFF_NAME_LENS + index * 2;
        u16::from_le_bytes(self.bytes.as_slice()[off..off + 2].try_into().unwrap()) as usize
    }

    fn header_len(&self) -> usize {
        OFF_NAME_LENS + self.field_count() * 2
    }

    fn variant_start(&self) -> usize {
        self.header_len()
    }

    fn name_blob_start(&self) -> usize {
        self.variant_start() + self.variant_len()
    }

    fn total_name_bytes(&self) -> usize {
        (0..self.field_count()).map(|i| self.name_len(i)).sum()
    }

    fn value_slots_start(&self) -> usize {
        self.name_blob_start() + self.total_name_bytes()
    }

    fn field_name_at(&self, index: usize) -> &str {
        let mut offset = self.name_blob_start();
        for i in 0..index {
            offset += self.name_len(i);
        }
        let len = self.name_len(index);
        std::str::from_utf8(&self.bytes.as_slice()[offset..offset + len])
            .expect("RallocRecord: invalid UTF-8 in field name")
    }

    fn find_index(&self, name: &str) -> Option<usize> {
        (0..self.field_count()).find(|&i| self.field_name_at(i) == name)
    }

    pub(crate) fn len(&self) -> usize {
        self.field_count()
    }

    /// Real allocated size in bytes — for `heap_object_bytes` accounting.
    pub(crate) fn byte_len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn tag(&self) -> u32 {
        u32::from_le_bytes(
            self.bytes.as_slice()[OFF_TAG..OFF_TAG + 4]
                .try_into()
                .unwrap(),
        )
    }

    /// Empty string for a plain `Struct`/`Record`.
    pub(crate) fn variant(&self) -> &str {
        let start = self.variant_start();
        let len = self.variant_len();
        std::str::from_utf8(&self.bytes.as_slice()[start..start + len])
            .expect("RallocRecord: invalid UTF-8 in variant name")
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.find_index(name).is_some()
    }

    pub(crate) fn get(&self, name: &str) -> Option<Value> {
        let index = self.find_index(name)?;
        let start = self.value_slots_start() + index * ENCODED_VALUE_BYTES;
        let bytes = &self.bytes.as_slice()[start..start + ENCODED_VALUE_BYTES];
        Some(value_codec::decode_value(bytes.try_into().unwrap()))
    }

    /// Overwrites the value at field `name`. Returns `Ok(false)` if no such
    /// field exists (fields are never added after construction).
    pub(crate) fn set(&mut self, name: &str, value: &Value) -> Result<bool> {
        let Some(index) = self.find_index(name) else {
            return Ok(false);
        };
        let encoded = value_codec::encode_value(value)?;
        let start = self.value_slots_start() + index * ENCODED_VALUE_BYTES;
        self.bytes.as_mut_slice()[start..start + ENCODED_VALUE_BYTES].copy_from_slice(&encoded);
        Ok(true)
    }

    /// Decodes every `(name, value)` pair, in construction order.
    pub(crate) fn fields(&self) -> Vec<(String, Value)> {
        (0..self.field_count())
            .map(|i| {
                let name = self.field_name_at(i).to_owned();
                let start = self.value_slots_start() + i * ENCODED_VALUE_BYTES;
                let value = value_codec::decode_value(
                    self.bytes.as_slice()[start..start + ENCODED_VALUE_BYTES]
                        .try_into()
                        .unwrap(),
                );
                (name, value)
            })
            .collect()
    }
}

impl std::fmt::Debug for RallocRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RallocRecord")
            .field("variant", &self.variant())
            .field("field_count", &self.field_count())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::value::{CastKind, PointerKind};

    #[test]
    fn struct_get_set_and_fields_round_trip() {
        let fields = vec![
            ("x".to_string(), Value::I64(1)),
            ("y".to_string(), Value::I64(2)),
        ];
        let mut record = RallocRecord::new(0, "", &fields).unwrap();
        assert_eq!(record.len(), 2);
        assert_eq!(record.tag(), 0);
        assert_eq!(record.variant(), "");
        assert_eq!(record.get("x"), Some(Value::I64(1)));
        assert_eq!(record.get("y"), Some(Value::I64(2)));
        assert_eq!(record.get("z"), None);

        assert!(record.set("y", &Value::I64(99)).unwrap());
        assert_eq!(record.get("y"), Some(Value::I64(99)));
        assert!(!record.set("nope", &Value::I64(0)).unwrap());

        assert_eq!(
            record.fields(),
            vec![
                ("x".to_string(), Value::I64(1)),
                ("y".to_string(), Value::I64(99)),
            ]
        );
    }

    #[test]
    fn enum_tag_and_variant_round_trip() {
        let fields = vec![("value".to_string(), Value::I64(42))];
        let record = RallocRecord::new(7, "Some", &fields).unwrap();
        assert_eq!(record.tag(), 7);
        assert_eq!(record.variant(), "Some");
        assert_eq!(record.get("value"), Some(Value::I64(42)));
    }

    #[test]
    fn empty_fields_record_works() {
        let record = RallocRecord::new(0, "None", &[]).unwrap();
        assert_eq!(record.len(), 0);
        assert_eq!(record.variant(), "None");
        assert_eq!(record.fields(), vec![]);
    }

    #[test]
    fn field_values_can_be_pointers_and_heap_refs() {
        let fields = vec![(
            "p".to_string(),
            Value::Pointer(crate::RawPointer::new(
                5,
                PointerKind::Array,
                2,
                "",
                1,
                CastKind::None,
            )),
        )];
        let record = RallocRecord::new(0, "", &fields).unwrap();
        assert_eq!(record.get("p"), Some(fields[0].1.clone()));
    }
}
