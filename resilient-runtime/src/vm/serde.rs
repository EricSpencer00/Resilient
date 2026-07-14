//! RES-3987 (D-E1): `#![no_std]`, zero-heap, zero-panic bytecode
//! serialization for [`super::Instr`] — the `.rzbc` wire format a
//! thin on-device loader will consume.
//!
//! `docs/EMBEDDED_PIPELINE.md` section 3.2 sketches the eventual
//! host-side `Program`/`Op` artifact format (magic + version +
//! const pool + function table). This module is the no_std-side
//! counterpart for the scalar [`super::Instr`] subset that
//! [`super::Vm`] already executes: a compact tagged encoding for a
//! flat `&[Instr]` stream, with no const pool or function table
//! (those are host `Program` concepts this module's `Instr` slice
//! doesn't have — every operand already lives inline on the
//! instruction itself). A later PR unifies the two once the host
//! and embedded instruction sets converge.
//!
//! # Wire format
//!
//! ```text
//! [4] magic:       b"RZBC"
//! [2] version:     u16 LE (currently 1)
//! [4] instr_count: u32 LE
//! [N] instructions, each:
//!       [1] tag: u8            (see the tag table below)
//!       [.] operand, tag-dependent, always fixed-width for that tag:
//!             none                       — 0 bytes
//!             u16 LE (local index)       — 2 bytes
//!             u32 LE (jump target)       — 4 bytes
//!             Value                      — 1 value-tag byte + payload:
//!                 0 = Int(i64)   → 8 bytes LE
//!                 1 = Bool(bool) → 1 byte (0 or 1)
//!                 2 = Float(f64) → 8 bytes LE (`f64::to_bits`)
//! ```
//!
//! Every instruction's total width is determined solely by its tag
//! byte — no length prefixes, so a malformed length can never walk
//! a no_std decoder off the end of the buffer. This mirrors the
//! "no variable-length instruction decode" rationale in the design
//! doc's artifact-format section.
//!
//! # No-panic guarantee
//!
//! [`encode`] and [`decode`] never panic on any input, valid or
//! not: every byte read/write is bounds-checked and every error
//! path returns a typed [`EncodeError`]/[`DecodeError`] instead of
//! indexing, unwrapping, or asserting. Malformed or truncated input
//! (including a byte-for-byte-random buffer) always yields a
//! `Result`, never a crash — see the `decode_never_panics_on_random_bytes`
//! fuzz-style test below.
//!
//! ```
//! use resilient_runtime::vm::{Instr, Value};
//! use resilient_runtime::vm::serde::{decode, encode};
//!
//! let program = [
//!     Instr::PushConst(Value::Int(1)),
//!     Instr::PushConst(Value::Int(2)),
//!     Instr::Add,
//!     Instr::Return,
//! ];
//! let mut buf = [0u8; 64];
//! let len = encode(&program, &mut buf).unwrap();
//!
//! let mut out = [Instr::Return; 8];
//! let count = decode(&buf[..len], &mut out).unwrap();
//! assert_eq!(&out[..count], &program[..]);
//! ```

use super::{Instr, Value};

/// Wire-format magic bytes identifying a `.rzbc` blob.
pub const MAGIC: [u8; 4] = *b"RZBC";

/// Current wire-format version. Bump on any incompatible layout
/// change; [`decode`] rejects anything else with
/// [`DecodeError::UnsupportedVersion`].
pub const FORMAT_VERSION: u16 = 1;

/// Byte length of the fixed header (`magic` + `version` +
/// `instr_count`).
pub const HEADER_LEN: usize = 4 + 2 + 4;

const TAG_PUSH_CONST: u8 = 0;
const TAG_LOAD_LOCAL: u8 = 1;
const TAG_STORE_LOCAL: u8 = 2;
const TAG_ADD: u8 = 3;
const TAG_SUB: u8 = 4;
const TAG_MUL: u8 = 5;
const TAG_DIV: u8 = 6;
const TAG_REM: u8 = 7;
const TAG_NEG: u8 = 8;
const TAG_EQ: u8 = 9;
const TAG_NEQ: u8 = 10;
const TAG_LT: u8 = 11;
const TAG_LE: u8 = 12;
const TAG_GT: u8 = 13;
const TAG_GE: u8 = 14;
const TAG_NOT: u8 = 15;
const TAG_JUMP: u8 = 16;
const TAG_JUMP_IF_FALSE: u8 = 17;
const TAG_JUMP_IF_TRUE: u8 = 18;
const TAG_RETURN: u8 = 19;

const VALUE_TAG_INT: u8 = 0;
const VALUE_TAG_BOOL: u8 = 1;
const VALUE_TAG_FLOAT: u8 = 2;

/// Errors [`encode`] can return. Every fallible write path returns
/// one of these instead of panicking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    /// The output buffer ran out of room before the whole program
    /// (header + every instruction) was written.
    BufferTooSmall,
}

/// Errors [`decode`] can return. Every fallible read/validation
/// path returns one of these instead of panicking — see the
/// module-level "No-panic guarantee" section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// The input ended before a complete header or instruction
    /// could be read.
    Truncated,
    /// The first 4 bytes were not [`MAGIC`].
    BadMagic,
    /// The header's `version` field is not [`FORMAT_VERSION`].
    UnsupportedVersion,
    /// The header's `instr_count` exceeds the caller-provided
    /// output slice's capacity.
    TooManyInstrs,
    /// An instruction's tag byte did not match any known opcode.
    /// The payload is the offending byte.
    BadTag(u8),
    /// An operand was structurally present but held an invalid
    /// value for its type (e.g. a `Value` tag byte outside 0..=2,
    /// or a bool byte outside 0..=1).
    BadOperand,
}

struct Writer<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> Writer<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), EncodeError> {
        let end = self
            .pos
            .checked_add(bytes.len())
            .ok_or(EncodeError::BufferTooSmall)?;
        let slot = self
            .buf
            .get_mut(self.pos..end)
            .ok_or(EncodeError::BufferTooSmall)?;
        slot.copy_from_slice(bytes);
        self.pos = end;
        Ok(())
    }

    fn write_u8(&mut self, v: u8) -> Result<(), EncodeError> {
        self.write_bytes(&[v])
    }

    fn write_u16(&mut self, v: u16) -> Result<(), EncodeError> {
        self.write_bytes(&v.to_le_bytes())
    }

    fn write_u32(&mut self, v: u32) -> Result<(), EncodeError> {
        self.write_bytes(&v.to_le_bytes())
    }

    fn write_i64(&mut self, v: i64) -> Result<(), EncodeError> {
        self.write_bytes(&v.to_le_bytes())
    }

    fn write_f64(&mut self, v: f64) -> Result<(), EncodeError> {
        self.write_bytes(&v.to_bits().to_le_bytes())
    }

    fn write_value(&mut self, v: Value) -> Result<(), EncodeError> {
        match v {
            Value::Int(i) => {
                self.write_u8(VALUE_TAG_INT)?;
                self.write_i64(i)
            }
            Value::Bool(b) => {
                self.write_u8(VALUE_TAG_BOOL)?;
                self.write_u8(b as u8)
            }
            Value::Float(f) => {
                self.write_u8(VALUE_TAG_FLOAT)?;
                self.write_f64(f)
            }
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n).ok_or(DecodeError::Truncated)?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or(DecodeError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn read_u8(&mut self) -> Result<u8, DecodeError> {
        let b = self.read_bytes(1)?;
        b.first().copied().ok_or(DecodeError::Truncated)
    }

    fn read_u16(&mut self) -> Result<u16, DecodeError> {
        let b = self.read_bytes(2)?;
        let arr: [u8; 2] = match b.try_into() {
            Ok(a) => a,
            Err(_) => return Err(DecodeError::Truncated),
        };
        Ok(u16::from_le_bytes(arr))
    }

    fn read_u32(&mut self) -> Result<u32, DecodeError> {
        let b = self.read_bytes(4)?;
        let arr: [u8; 4] = match b.try_into() {
            Ok(a) => a,
            Err(_) => return Err(DecodeError::Truncated),
        };
        Ok(u32::from_le_bytes(arr))
    }

    fn read_i64(&mut self) -> Result<i64, DecodeError> {
        let b = self.read_bytes(8)?;
        let arr: [u8; 8] = match b.try_into() {
            Ok(a) => a,
            Err(_) => return Err(DecodeError::Truncated),
        };
        Ok(i64::from_le_bytes(arr))
    }

    fn read_f64(&mut self) -> Result<f64, DecodeError> {
        let b = self.read_bytes(8)?;
        let arr: [u8; 8] = match b.try_into() {
            Ok(a) => a,
            Err(_) => return Err(DecodeError::Truncated),
        };
        Ok(f64::from_bits(u64::from_le_bytes(arr)))
    }

    fn read_value(&mut self) -> Result<Value, DecodeError> {
        match self.read_u8()? {
            VALUE_TAG_INT => Ok(Value::Int(self.read_i64()?)),
            VALUE_TAG_BOOL => match self.read_u8()? {
                0 => Ok(Value::Bool(false)),
                1 => Ok(Value::Bool(true)),
                _ => Err(DecodeError::BadOperand),
            },
            VALUE_TAG_FLOAT => Ok(Value::Float(self.read_f64()?)),
            _ => Err(DecodeError::BadOperand),
        }
    }
}

/// Encode `program` into `out`, returning the number of bytes
/// written. Never panics: a buffer too small to hold the header
/// plus every instruction yields [`EncodeError::BufferTooSmall`]
/// rather than an out-of-bounds write.
pub fn encode(program: &[Instr], out: &mut [u8]) -> Result<usize, EncodeError> {
    let instr_count: u32 = match u32::try_from(program.len()) {
        Ok(n) => n,
        Err(_) => return Err(EncodeError::BufferTooSmall),
    };

    let mut w = Writer::new(out);
    w.write_bytes(&MAGIC)?;
    w.write_u16(FORMAT_VERSION)?;
    w.write_u32(instr_count)?;

    for instr in program {
        match *instr {
            Instr::PushConst(v) => {
                w.write_u8(TAG_PUSH_CONST)?;
                w.write_value(v)?;
            }
            Instr::LoadLocal(idx) => {
                w.write_u8(TAG_LOAD_LOCAL)?;
                w.write_u16(idx)?;
            }
            Instr::StoreLocal(idx) => {
                w.write_u8(TAG_STORE_LOCAL)?;
                w.write_u16(idx)?;
            }
            Instr::Add => w.write_u8(TAG_ADD)?,
            Instr::Sub => w.write_u8(TAG_SUB)?,
            Instr::Mul => w.write_u8(TAG_MUL)?,
            Instr::Div => w.write_u8(TAG_DIV)?,
            Instr::Rem => w.write_u8(TAG_REM)?,
            Instr::Neg => w.write_u8(TAG_NEG)?,
            Instr::Eq => w.write_u8(TAG_EQ)?,
            Instr::Neq => w.write_u8(TAG_NEQ)?,
            Instr::Lt => w.write_u8(TAG_LT)?,
            Instr::Le => w.write_u8(TAG_LE)?,
            Instr::Gt => w.write_u8(TAG_GT)?,
            Instr::Ge => w.write_u8(TAG_GE)?,
            Instr::Not => w.write_u8(TAG_NOT)?,
            Instr::Jump(target) => {
                w.write_u8(TAG_JUMP)?;
                w.write_u32(target)?;
            }
            Instr::JumpIfFalse(target) => {
                w.write_u8(TAG_JUMP_IF_FALSE)?;
                w.write_u32(target)?;
            }
            Instr::JumpIfTrue(target) => {
                w.write_u8(TAG_JUMP_IF_TRUE)?;
                w.write_u32(target)?;
            }
            Instr::Return => w.write_u8(TAG_RETURN)?,
        }
    }

    Ok(w.pos)
}

/// Decode a `.rzbc` blob from `bytes` into `out`, returning the
/// number of instructions written to `out[..n]`. Never panics on
/// any input — truncated, corrupt, or adversarially mutated bytes
/// always produce a typed [`DecodeError`] instead of a crash. See
/// the module-level "No-panic guarantee" section.
pub fn decode(bytes: &[u8], out: &mut [Instr]) -> Result<usize, DecodeError> {
    let mut r = Reader::new(bytes);

    let magic = r.read_bytes(4)?;
    if magic != MAGIC {
        return Err(DecodeError::BadMagic);
    }

    let version = r.read_u16()?;
    if version != FORMAT_VERSION {
        return Err(DecodeError::UnsupportedVersion);
    }

    let instr_count = r.read_u32()? as usize;
    if instr_count > out.len() {
        return Err(DecodeError::TooManyInstrs);
    }

    for slot in out.iter_mut().take(instr_count) {
        let tag = r.read_u8()?;
        *slot = match tag {
            TAG_PUSH_CONST => Instr::PushConst(r.read_value()?),
            TAG_LOAD_LOCAL => Instr::LoadLocal(r.read_u16()?),
            TAG_STORE_LOCAL => Instr::StoreLocal(r.read_u16()?),
            TAG_ADD => Instr::Add,
            TAG_SUB => Instr::Sub,
            TAG_MUL => Instr::Mul,
            TAG_DIV => Instr::Div,
            TAG_REM => Instr::Rem,
            TAG_NEG => Instr::Neg,
            TAG_EQ => Instr::Eq,
            TAG_NEQ => Instr::Neq,
            TAG_LT => Instr::Lt,
            TAG_LE => Instr::Le,
            TAG_GT => Instr::Gt,
            TAG_GE => Instr::Ge,
            TAG_NOT => Instr::Not,
            TAG_JUMP => Instr::Jump(r.read_u32()?),
            TAG_JUMP_IF_FALSE => Instr::JumpIfFalse(r.read_u32()?),
            TAG_JUMP_IF_TRUE => Instr::JumpIfTrue(r.read_u32()?),
            TAG_RETURN => Instr::Return,
            other => return Err(DecodeError::BadTag(other)),
        };
    }

    Ok(instr_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::Vm;

    fn roundtrip(program: &[Instr]) -> ([Instr; 32], usize) {
        let mut buf = [0u8; 512];
        let len = encode(program, &mut buf).expect("encode should fit in 512 bytes");
        let mut out = [Instr::Return; 32];
        let count = decode(&buf[..len], &mut out).expect("decode should succeed");
        (out, count)
    }

    // ---------- round-trip ----------

    #[test]
    fn roundtrip_empty_program() {
        let program: [Instr; 0] = [];
        let (_out, count) = roundtrip(&program);
        assert_eq!(count, 0);
    }

    #[test]
    fn roundtrip_arithmetic_precedence_program() {
        let program = [
            Instr::PushConst(Value::Int(1)),
            Instr::PushConst(Value::Int(2)),
            Instr::PushConst(Value::Int(3)),
            Instr::Mul,
            Instr::Add,
            Instr::Return,
        ];
        let (out, count) = roundtrip(&program);
        assert_eq!(&out[..count], &program[..]);
    }

    #[test]
    fn roundtrip_all_value_variants() {
        let program = [
            Instr::PushConst(Value::Int(-42)),
            Instr::PushConst(Value::Bool(true)),
            Instr::PushConst(Value::Bool(false)),
            Instr::PushConst(Value::Float(3.5)),
            Instr::PushConst(Value::Float(f64::NAN)),
            Instr::PushConst(Value::Int(i64::MIN)),
            Instr::PushConst(Value::Int(i64::MAX)),
            Instr::Return,
        ];
        let (out, count) = roundtrip(&program);
        assert_eq!(count, program.len());
        for (a, b) in out[..count].iter().zip(program.iter()) {
            match (a, b) {
                (Instr::PushConst(Value::Float(x)), Instr::PushConst(Value::Float(y)))
                    if x.is_nan() && y.is_nan() => {}
                _ => assert_eq!(a, b),
            }
        }
    }

    #[test]
    fn roundtrip_all_opcodes() {
        let program = [
            Instr::PushConst(Value::Int(1)),
            Instr::LoadLocal(3),
            Instr::StoreLocal(7),
            Instr::Add,
            Instr::Sub,
            Instr::Mul,
            Instr::Div,
            Instr::Rem,
            Instr::Neg,
            Instr::Eq,
            Instr::Neq,
            Instr::Lt,
            Instr::Le,
            Instr::Gt,
            Instr::Ge,
            Instr::Not,
            Instr::Jump(0),
            Instr::JumpIfFalse(1),
            Instr::JumpIfTrue(2),
            Instr::Return,
        ];
        let (out, count) = roundtrip(&program);
        assert_eq!(&out[..count], &program[..]);
    }

    #[test]
    fn roundtrip_loop_program_and_it_still_executes_correctly() {
        let program = [
            Instr::PushConst(Value::Int(0)),
            Instr::StoreLocal(0),
            Instr::PushConst(Value::Int(0)),
            Instr::StoreLocal(1),
            Instr::LoadLocal(0),
            Instr::PushConst(Value::Int(5)),
            Instr::Lt,
            Instr::JumpIfFalse(17),
            Instr::LoadLocal(1),
            Instr::LoadLocal(0),
            Instr::Add,
            Instr::StoreLocal(1),
            Instr::LoadLocal(0),
            Instr::PushConst(Value::Int(1)),
            Instr::Add,
            Instr::StoreLocal(0),
            Instr::Jump(4),
            Instr::LoadLocal(1),
            Instr::Return,
        ];
        let (out, count) = roundtrip(&program);
        assert_eq!(&out[..count], &program[..]);

        let mut vm = Vm::<8, 2>::new();
        assert_eq!(vm.run(&out[..count]), Ok(Value::Int(1 + 2 + 3 + 4)));
    }

    #[test]
    fn roundtrip_max_local_and_jump_operands() {
        let program = [
            Instr::LoadLocal(u16::MAX),
            Instr::Jump(u32::MAX),
            Instr::Return,
        ];
        let (out, count) = roundtrip(&program);
        assert_eq!(&out[..count], &program[..]);
    }

    // ---------- encode errors ----------

    #[test]
    fn encode_into_undersized_buffer_is_buffer_too_small_not_a_panic() {
        let program = [Instr::PushConst(Value::Int(1)), Instr::Return];
        let mut buf = [0u8; 3];
        assert_eq!(encode(&program, &mut buf), Err(EncodeError::BufferTooSmall));
    }

    #[test]
    fn encode_into_zero_length_buffer_is_buffer_too_small_not_a_panic() {
        let program = [Instr::Return];
        let mut buf: [u8; 0] = [];
        assert_eq!(encode(&program, &mut buf), Err(EncodeError::BufferTooSmall));
    }

    #[test]
    fn encode_empty_program_into_header_sized_buffer_succeeds() {
        let program: [Instr; 0] = [];
        let mut buf = [0u8; HEADER_LEN];
        assert_eq!(encode(&program, &mut buf), Ok(HEADER_LEN));
    }

    // ---------- decode errors: never panic, always Result::Err ----------

    #[test]
    fn decode_empty_input_is_truncated_not_a_panic() {
        let mut out = [Instr::Return; 4];
        assert_eq!(decode(&[], &mut out), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_truncated_header_is_truncated_not_a_panic() {
        let mut out = [Instr::Return; 4];
        assert_eq!(decode(&MAGIC[..2], &mut out), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_bad_magic_is_bad_magic_not_a_panic() {
        let mut buf = [0u8; HEADER_LEN];
        let mut w = Writer::new(&mut buf);
        w.write_bytes(b"NOPE").unwrap();
        w.write_u16(FORMAT_VERSION).unwrap();
        w.write_u32(0).unwrap();
        let mut out = [Instr::Return; 4];
        assert_eq!(decode(&buf, &mut out), Err(DecodeError::BadMagic));
    }

    #[test]
    fn decode_unsupported_version_is_typed_error_not_a_panic() {
        let mut buf = [0u8; HEADER_LEN];
        let mut w = Writer::new(&mut buf);
        w.write_bytes(&MAGIC).unwrap();
        w.write_u16(FORMAT_VERSION.wrapping_add(1)).unwrap();
        w.write_u32(0).unwrap();
        let mut out = [Instr::Return; 4];
        assert_eq!(decode(&buf, &mut out), Err(DecodeError::UnsupportedVersion));
    }

    #[test]
    fn decode_instr_count_exceeding_output_capacity_is_typed_error_not_a_panic() {
        let program = [Instr::Return, Instr::Return, Instr::Return];
        let mut buf = [0u8; 64];
        let len = encode(&program, &mut buf).unwrap();
        let mut out = [Instr::Return; 2];
        assert_eq!(
            decode(&buf[..len], &mut out),
            Err(DecodeError::TooManyInstrs)
        );
    }

    #[test]
    fn decode_instr_count_lying_huge_is_too_many_instrs_not_a_panic() {
        let mut buf = [0u8; HEADER_LEN];
        let mut w = Writer::new(&mut buf);
        w.write_bytes(&MAGIC).unwrap();
        w.write_u16(FORMAT_VERSION).unwrap();
        w.write_u32(1_000_000).unwrap();
        let mut out = [Instr::Return; 4];
        // instr_count (1_000_000) > out.len() (4) is checked before
        // any per-instruction read is attempted.
        assert_eq!(decode(&buf, &mut out), Err(DecodeError::TooManyInstrs));
    }

    #[test]
    fn decode_bad_tag_is_typed_error_not_a_panic() {
        let mut buf = [0u8; HEADER_LEN + 1];
        {
            let mut w = Writer::new(&mut buf);
            w.write_bytes(&MAGIC).unwrap();
            w.write_u16(FORMAT_VERSION).unwrap();
            w.write_u32(1).unwrap();
            w.write_u8(200).unwrap();
        }
        let mut out = [Instr::Return; 4];
        assert_eq!(decode(&buf, &mut out), Err(DecodeError::BadTag(200)));
    }

    #[test]
    fn decode_bad_value_tag_is_bad_operand_not_a_panic() {
        let mut buf = [0u8; HEADER_LEN + 2];
        {
            let mut w = Writer::new(&mut buf);
            w.write_bytes(&MAGIC).unwrap();
            w.write_u16(FORMAT_VERSION).unwrap();
            w.write_u32(1).unwrap();
            w.write_u8(TAG_PUSH_CONST).unwrap();
            w.write_u8(99).unwrap();
        }
        let mut out = [Instr::Return; 4];
        assert_eq!(decode(&buf, &mut out), Err(DecodeError::BadOperand));
    }

    #[test]
    fn decode_bad_bool_byte_is_bad_operand_not_a_panic() {
        let mut buf = [0u8; HEADER_LEN + 3];
        {
            let mut w = Writer::new(&mut buf);
            w.write_bytes(&MAGIC).unwrap();
            w.write_u16(FORMAT_VERSION).unwrap();
            w.write_u32(1).unwrap();
            w.write_u8(TAG_PUSH_CONST).unwrap();
            w.write_u8(VALUE_TAG_BOOL).unwrap();
            w.write_u8(7).unwrap();
        }
        let mut out = [Instr::Return; 4];
        assert_eq!(decode(&buf, &mut out), Err(DecodeError::BadOperand));
    }

    #[test]
    fn decode_truncated_mid_operand_is_truncated_not_a_panic() {
        let program = [Instr::PushConst(Value::Int(42)), Instr::Return];
        let mut buf = [0u8; 64];
        encode(&program, &mut buf).unwrap();
        let mut out = [Instr::Return; 4];
        // Cut off partway through the Int(i64) payload.
        assert_eq!(
            decode(&buf[..HEADER_LEN + 2], &mut out),
            Err(DecodeError::Truncated)
        );
    }

    #[test]
    fn decode_truncated_mid_tag_stream_is_truncated_not_a_panic() {
        let program = [Instr::Return, Instr::Return];
        let mut buf = [0u8; 64];
        let len = encode(&program, &mut buf).unwrap();
        let mut out = [Instr::Return; 4];
        assert_eq!(
            decode(&buf[..len - 1], &mut out),
            Err(DecodeError::Truncated)
        );
    }

    // ---------- fuzz-style: never panic on arbitrary/mutated bytes ----------

    /// Deterministic xorshift32 PRNG — avoids pulling in a `rand`
    /// dependency for a no_std crate just to mutate test bytes.
    struct Xorshift32(u32);

    impl Xorshift32 {
        fn next(&mut self) -> u32 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.0 = x;
            x
        }
    }

    #[test]
    fn decode_never_panics_on_random_bytes() {
        let mut rng = Xorshift32(0xC0FF_EE01);
        let mut out = [Instr::Return; 16];
        for len in 0..40 {
            let mut buf = [0u8; 40];
            for slot in buf.iter_mut().take(len) {
                *slot = (rng.next() & 0xFF) as u8;
            }
            // Only the assertion of interest is that this call
            // returns instead of panicking; both Ok and Err are
            // acceptable outcomes for random input.
            let _ = decode(&buf[..len], &mut out);
        }
    }

    #[test]
    fn decode_never_panics_on_mutated_valid_encoding() {
        let program = [
            Instr::PushConst(Value::Int(7)),
            Instr::PushConst(Value::Float(2.5)),
            Instr::Add,
            Instr::LoadLocal(1),
            Instr::JumpIfFalse(0),
            Instr::Return,
        ];
        let mut buf = [0u8; 64];
        let len = encode(&program, &mut buf).unwrap();

        let mut rng = Xorshift32(0xDEAD_BEEF);
        let mut out = [Instr::Return; 16];
        for _ in 0..2000 {
            let mut mutated = buf;
            let flips = 1 + (rng.next() as usize % 4);
            for _ in 0..flips {
                let idx = rng.next() as usize % len;
                let bit = 1u8 << (rng.next() % 8) as u8;
                if let Some(byte) = mutated.get_mut(idx) {
                    *byte ^= bit;
                }
            }
            let _ = decode(&mutated[..len], &mut out);
        }
    }

    #[test]
    fn encode_never_panics_on_undersized_buffers() {
        let program = [
            Instr::PushConst(Value::Int(1)),
            Instr::PushConst(Value::Float(1.0)),
            Instr::PushConst(Value::Bool(true)),
            Instr::Add,
            Instr::Jump(0),
            Instr::Return,
        ];
        let mut buf = [0u8; HEADER_LEN + 40];
        for cap in 0..buf.len() {
            let _ = encode(&program, &mut buf[..cap]);
        }
    }
}
