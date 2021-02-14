//! Debug formatting for mm0-rs items that works around indirection.
//!
//! Meant to be used in conjunction with a [`FormatEnv`] struct. Can be used
//! with the `{:#?}` format specifier as in the following example:
//! ```ignore
//! let fe = FormatEnv { source: &text, env };
//! let thm: Thm = /* some theorem */;
//! println!("{:#?}", fe.to(&thm));
//! ```
//! You can use the regular `{:?}` debug format specifier, but the formatting
//! will be a little bit squirrely.
//!
//! Implementations for native rust types and mm0-rs types that do not use indirection
//! are generated by `macro_rules` macros. Implementations for indirect `mm0-rs` types
//! are generated by the [`EnvDebug`] and [`EnvDebugPub`] macros
use super::{print::FormatEnv, super::environment::{AtomId, SortId, TermId, ThmId} };

/// Companion to [`EnvDisplay`](super::print::EnvDisplay)
pub trait EnvDebug {
  /// Get the actual debug representation. It's highly unlikely you'll
  /// need to call this outside of another [`EnvDebug`] implementation.
  fn env_dbg<'a>(&self, fe: FormatEnv<'a>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result;
}


// For types external to mm0-rs, generate an instance of EnvDebug that just returns its default
// std::fmt::Debug representation using the {:#?} formatter.
macro_rules! env_debug {
  ( $($xs:ty),+ ) => {
    $(
      impl EnvDebug for $xs {
        fn env_dbg<'a>(&self, _: FormatEnv<'a>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
          write!(f, "{:#?}", self)
        }
      }
    )+
  };
}

// Generate an implementation for any sequence whose `self.iter()` method has an associated
// Item type that implements EnvDebug.
//
// Type parameters need to be in a comma separated list that's surrounded by parens.
macro_rules! env_debug_seq {
  ( $( ($($id:ident),+) -> $T:ty )+ ) => {
    $(
      impl<$($id: EnvDebug),+> EnvDebug for $T {
        fn env_dbg<'a>(&self, fe: FormatEnv<'a>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
          f.debug_list().entries(self.iter().map(|x| fe.to(x))).finish()
        }
      }
    )+
  };
}

// Generate an implementation for any map type whose `self.iter()` method has an associated
// Item which is a (&K, &V), where the K and V types both implement EnvDebug.
//
// Type parameters need to be in a comma separated list that's surrounded by parens.
macro_rules! env_debug_map {
  ( $( ($($id:ident),+) -> $T:ty )+ ) => {
    $(
      impl<$($id: EnvDebug),+> EnvDebug for $T {
        fn env_dbg<'a>(&self, fe: FormatEnv<'a>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
          f.debug_map().entries(
            self.iter().map(|(k, v)| (fe.to(k), fe.to(v)))
          ).finish()
        }
      }
    )+
  };
}

// Generate an implementation for some type whose AsRef target implements EnvDebug, like Box<A>.
//
// Type parameters need to be in a comma separated list that's surrounded by parens.
macro_rules! env_debug_as_ref {
  ( $( ($($id:ident),+) -> $T:ty )+ ) => {
    $(
      impl<$($id: EnvDebug),+> EnvDebug for $T {
        fn env_dbg<'a>(&self, fe: FormatEnv<'a>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
          self.as_ref().env_dbg(fe, f)
        }
      }
    )+
  };
}


// Generate implementations of EnvDebug for arrays of a type that implements EnvDebug.
macro_rules! dbg_arrays {
  ($($N:literal)+) => {
    $(
      impl<A: EnvDebug> EnvDebug for [A; $N] {
        fn env_dbg<'a>(&self, fe: FormatEnv<'a>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
          self.as_ref().env_dbg(fe, f)
        }
      }
    )+
  }
}

// Generate implementations of EnvDebug for tuples with type parameters that implement EnvDebug.
macro_rules! dbg_tuples {
  ($( { $( ($idx:tt) -> $T:ident)+ } )+) => {
    $(
       impl<$($T: EnvDebug),+> EnvDebug for ($($T),+) {
         fn env_dbg<'a>(&self, fe: FormatEnv<'a>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
           let mut base = f.debug_tuple("");
           $(
             base.field(&fe.to(&(self.$idx)));
           )+
           base.finish()
         }
       }
    )+
  }
}

// Generate implementations for SortId, ThmId, and TermId
// that show the index, and the name.
macro_rules! env_debug_id {
  ( $(($x:ident, $loc:ident))+ ) => {
    $(
      impl EnvDebug for $x {
        fn env_dbg<'a>(&self, fe: FormatEnv<'a>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
          let mut base = f.debug_tuple(stringify!($x));
          match self {
            $x(idx) => { base.field(&fe.to(idx)); }
          }

          let atom_id = &fe.$loc[*self].atom;
          let atom_name = &(fe.data[*atom_id].name);
          base.field(&fe.to(atom_name));
          base.finish()
        }
      }
    )+
  };
}


// Instances for a few common types that require some sort of special behavior to display nicely.
impl<A: EnvDebug> EnvDebug for std::cell::RefCell<A> {
  fn env_dbg<'a>(&self, fe: FormatEnv<'a>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self.try_borrow() {
      Ok(x) => x.env_dbg(fe, f),
      Err(_) => write!(f, "_mutably borrowed RefCell_")
    }
  }
}

// using write directly with the regular debug formatter seems
// to be the nicest formatting option.
impl<A: EnvDebug, E: EnvDebug> EnvDebug for Result<A, E> {
  fn env_dbg<'a>(&self, fe: FormatEnv<'a>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "{:#?}",
      self.as_ref().map(|x| fe.to(x)).map_err(|e| fe.to(e))
    )
  }
}

// using write directly with the regular debug formatter seems
// to be the nicest formatting option.
impl<A: EnvDebug> EnvDebug for Option<A> {
  fn env_dbg<'a>(&self, fe: FormatEnv<'a>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "{:#?}",
      self.as_ref().map(|x| fe.to(x))
    )
  }
}

impl<A: EnvDebug + Copy> EnvDebug for std::cell::Cell<A> {
  fn env_dbg<'a>(&self, fe: FormatEnv<'a>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    self.get().env_dbg(fe, f)
  }
}

impl<A: EnvDebug + ?Sized> EnvDebug for std::sync::Arc<A> {
  fn env_dbg<'a>(&self, fe: FormatEnv<'a>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    std::sync::Arc::as_ref(self).env_dbg(fe, f)
  }
}

impl<A: EnvDebug + ?Sized> EnvDebug for std::sync::Weak<A> {
  fn env_dbg<'a>(&self, fe: FormatEnv<'a>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self.upgrade() {
      None => write!(f, "_Weak_"),
      Some(arc) => arc.env_dbg(fe, f)
    }
  }
}

impl<A: EnvDebug> EnvDebug for std::rc::Rc<A> {
  fn env_dbg<'a>(&self, fe: FormatEnv<'a>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    std::rc::Rc::as_ref(self).env_dbg(fe, f)
  }
}

impl<A: EnvDebug + ?Sized> EnvDebug for std::rc::Weak<A> {
  fn env_dbg<'a>(&self, fe: FormatEnv<'a>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self.upgrade() {
      None => write!(f, "_Weak_"),
      Some(arc) => arc.env_dbg(fe, f)
    }
  }
}

// Needs a separate implementation since it doesn't have
// an `atom` field, and the others don't have `name` field.
impl EnvDebug for AtomId {
  fn env_dbg<'a>(&self, fe: FormatEnv<'a>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let mut base = f.debug_tuple("AtomID");
    match self {
      AtomId(idx) => {
        base.field(&fe.to(idx));
      }
    }
    let atom_name = &fe.data[*self].name;
    base.field(&fe.to(atom_name));
    base.finish()
  }
}

env_debug! {
  bool,
  u8,
  u16,
  u32,
  u64,
  usize,
  i8,
  i16,
  i32,
  i64,
  isize,
  f32,
  f64,
  str,
  String,
  std::path::PathBuf,
  std::sync::atomic::AtomicBool,
  num::BigInt,
  crate::util::ArcString,
  crate::elab::lisp::Syntax,
  crate::elab::lisp::BuiltinProc,
  crate::elab::lisp::ProcSpec,
  crate::parser::ast::Prec,
  crate::elab::environment::Literal,
  crate::parser::ast::Modifiers,
  crate::util::Span,
  crate::util::FileRef,
  crate::util::FileSpan
}

#[cfg(feature = "server")]
env_debug! {lsp_types::Url}

env_debug_seq! {
  (A) -> &[A]
  (A) -> Vec<A>
}

env_debug_map! {
  (K, V) -> std::collections::HashMap<K, V>
}

env_debug_as_ref! {
  (A) -> Box<A>
  (A) -> Box<[A]>
}

dbg_arrays! {
     0  1  2  3  4  5  6  7  8  9
    10 11 12 13 14 15 16 17 18 19
    20 21 22 23 24 25 26 27 28 29
    30 31 32
}

dbg_tuples! {
  {
    (0) -> A
    (1) -> B
  }
  {
    (0) -> A
    (1) -> B
    (2) -> C
  }
  {
    (0) -> A
    (1) -> B
    (2) -> C
    (3) -> D
  }
  {
    (0) -> A
    (1) -> B
    (2) -> C
    (3) -> D
    (4) -> E
  }
  {
    (0) -> A
    (1) -> B
    (2) -> C
    (3) -> D
    (4) -> E
    (5) -> F
  }
  {
    (0) -> A
    (1) -> B
    (2) -> C
    (3) -> D
    (4) -> E
    (5) -> F
    (6) -> G
  }
  {
    (0) -> A
    (1) -> B
    (2) -> C
    (3) -> D
    (4) -> E
    (5) -> F
    (6) -> G
    (7) -> H
  }
  {
    (0) -> A
    (1) -> B
    (2) -> C
    (3) -> D
    (4) -> E
    (5) -> F
    (6) -> G
    (7) -> H
    (8) -> I
  }
}

env_debug_id! {
  (SortId, sorts)
  (ThmId, thms)
  (TermId, terms)
}
