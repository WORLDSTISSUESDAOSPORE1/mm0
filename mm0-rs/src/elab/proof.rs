use std::rc::Rc;
use std::hash::Hash;
use std::mem;
use std::collections::{HashMap, hash_map::Entry};
use super::environment::{AtomID, Type};
use super::{LocalContext, ElabError, Result, Elaborator, Environment,
  SortID, TermID, ThmID, Expr, ExprNode, ProofNode, DeclKey};
use super::lisp::{LispVal, LispKind, Uncons, InferTarget, print::FormatEnv};
use super::local_context::{InferSort, try_get_span_from};
use crate::util::*;

pub struct NodeHasher<'a> {
  pub lc: &'a LocalContext,
  pub fe: FormatEnv<'a>,
  pub var_map: HashMap<AtomID, usize>,
  pub fsp: FileSpan,
}

impl<'a> NodeHasher<'a> {
  pub fn new(lc: &'a LocalContext, fe: FormatEnv<'a>, fsp: FileSpan) -> Self {
    let mut var_map = HashMap::new();
    for (i, &(_, a, _)) in lc.var_order.iter().enumerate() {
      if let Some(a) = a {var_map.insert(a, i);}
    }
    NodeHasher {lc, fe, var_map, fsp}
  }

  fn err(&self, e: &LispKind, msg: impl Into<BoxError>) -> ElabError {
    self.err_sp(e.fspan().as_ref(), msg)
  }

  fn err_sp(&self, fsp: Option<&FileSpan>, msg: impl Into<BoxError>) -> ElabError {
    ElabError::new_e(try_get_span_from(&self.fsp, fsp), msg)
  }
}

pub trait NodeHash: Hash + Eq + Sized + std::fmt::Debug {
  const VAR: fn(usize) -> Self;
  fn from<'a>(nh: &NodeHasher<'a>, fsp: Option<&FileSpan>, r: &LispVal,
    de: &mut Dedup<Self>) -> Result<std::result::Result<Self, usize>>;
}

#[derive(Debug)]
pub struct Dedup<H: NodeHash> {
  map: HashMap<Rc<H>, usize>,
  prev: HashMap<*const LispKind, usize>,
  pub vec: Vec<(Rc<H>, bool)>,
}

impl<H: NodeHash> Dedup<H> {
  pub fn new(nargs: usize) -> Dedup<H> {
    let vec: Vec<_> = (0..nargs).map(|i| (Rc::new(H::VAR(i)), true)).collect();
    Dedup {
      map: vec.iter().enumerate().map(|(i, (r, _))| (r.clone(), i)).collect(),
      prev: HashMap::new(),
      vec,
    }
  }

  pub fn add_direct(&mut self, v: H) -> usize {
    match self.map.entry(Rc::new(v)) {
      Entry::Vacant(e) => {
        let n = self.vec.len();
        self.vec.push((e.key().clone(), false));
        e.insert(n);
        n
      }
      Entry::Occupied(e) => {
        let &n = e.get();
        self.vec[n].1 = true;
        n
      }
    }
  }

  pub fn add(&mut self, p: *const LispKind, v: H) -> usize {
    let n = self.add_direct(v);
    self.prev.insert(p, n);
    n
  }

  pub fn dedup(&mut self, nh: &NodeHasher, e: &LispVal) -> Result<usize> {
    let r = e.unwrapped_arc();
    let p: *const _ = &*r;
    Ok(match self.prev.get(&p) {
      Some(&n) => {self.vec[n].1 = true; n}
      None => {
        let n = match H::from(nh, e.fspan().as_ref(), &r, self)? {
          Ok(v) => self.add_direct(v),
          Err(n) => n,
        };
        self.prev.insert(p, n); n
      }
    })
  }

  fn map_inj<T: NodeHash>(&self, mut f: impl FnMut(&H) -> T) -> Dedup<T> {
    let mut d = Dedup {
      map: HashMap::new(),
      prev: self.prev.clone(),
      vec: Vec::with_capacity(self.vec.len())
    };
    for (i, &(ref h, b)) in self.vec.iter().enumerate() {
      let t = Rc::new(f(h));
      d.map.insert(t.clone(), i);
      d.vec.push((t, b));
    }
    d
  }
}

pub trait Node: Sized + std::fmt::Debug {
  type Hash: NodeHash;
  const REF: fn(usize) -> Self;
  fn from(e: &Self::Hash, ids: &mut [Val<Self>]) -> Self;
}

#[derive(Debug)]
pub enum Val<T: Node> {Built(T), Ref(usize), Done}

impl<T: Node> Val<T> {
  pub fn take(&mut self) -> T {
    match mem::replace(self, Val::Done) {
      Val::Built(x) => x,
      Val::Ref(n) => {*self = Val::Ref(n); T::REF(n)}
      Val::Done => panic!("taking a value twice")
    }
  }
}

pub struct Builder<T: Node> {
  pub ids: Vec<Val<T>>,
  pub heap: Vec<T>,
}

impl Elaborator {
  pub fn to_builder<T: Node>(&self, de: &Dedup<T::Hash>) -> Result<Builder<T>> {
    let mut ids: Vec<Val<T>> = Vec::with_capacity(de.vec.len());
    let mut heap = Vec::new();
    for &(ref e, b) in &de.vec {
      let node = T::from(&e, &mut ids);
      if b {
        ids.push(Val::Ref(heap.len()));
        heap.push(node);
      } else {
        ids.push(Val::Built(node))
      }
    }
    Ok(Builder {ids, heap})
  }
}

#[derive(PartialEq, Eq, Hash, Debug)]
pub enum ExprHash {
  Var(usize),
  Dummy(AtomID, SortID),
  App(TermID, Vec<usize>),
}

impl NodeHash for ExprHash {
  const VAR: fn(usize) -> Self = Self::Var;
  fn from<'a>(nh: &NodeHasher<'a>, fsp: Option<&FileSpan>, r: &LispVal,
      de: &mut Dedup<Self>) -> Result<std::result::Result<Self, usize>> {
    Ok(Ok(match &**r {
      &LispKind::Atom(a) => match nh.var_map.get(&a) {
        Some(&i) => ExprHash::Var(i),
        None => match nh.lc.vars.get(&a) {
          Some(&(true, InferSort::Bound {sort})) => ExprHash::Dummy(a, sort),
          _ => Err(nh.err_sp(fsp, format!("variable '{}' not found", nh.fe.data[a].name)))?,
        }
      },
      LispKind::MVar(_, tgt) => Err(nh.err_sp(fsp,
        format!("{}: {}", nh.fe.to(r), nh.fe.to(tgt))))?,
      _ => {
        let mut u = Uncons::from(r.clone());
        let head = u.next().ok_or_else(||
          nh.err_sp(fsp, format!("bad expression {}", nh.fe.to(r))))?;
        let a = head.as_atom().ok_or_else(|| nh.err(&head, "expected an atom"))?;
        let tid = nh.fe.term(a).ok_or_else(||
          nh.err(&head, format!("term '{}' not declared", nh.fe.data[a].name)))?;
        let mut ns = Vec::new();
        for e in &mut u { ns.push(de.dedup(nh, &e)?) }
        if !u.exactly(0) {Err(nh.err_sp(fsp, format!("bad expression {}", nh.fe.to(r))))?}
        ExprHash::App(tid, ns)
      }
    }))
  }
}

impl Node for ExprNode {
  type Hash = ExprHash;
  const REF: fn(usize) -> Self = ExprNode::Ref;
  fn from(e: &Self::Hash, ids: &mut [Val<Self>]) -> Self {
    match *e {
      ExprHash::Var(i) => ExprNode::Ref(i),
      ExprHash::Dummy(a, s) => ExprNode::Dummy(a, s),
      ExprHash::App(t, ref ns) => ExprNode::App(t,
        ns.iter().map(|&i| Val::take(&mut ids[i])).collect()),
    }
  }
}

impl Environment {
  pub fn deps(bvs: &[LispVal], mut v: Vec<LispVal>, xs: u64) -> Vec<LispVal> {
    v.push(if xs == 0 {LispVal::nil()} else {
      let mut i = 1;
      LispVal::list(bvs.iter().filter(|_| (xs & i != 0, i *= 2).0).cloned().collect())
    });
    v
  }

  pub fn binders(&self, bis: &[(Option<AtomID>, Type)],
      heap: &mut Vec<LispVal>, bvs: &mut Vec<LispVal>) -> LispVal {
    LispVal::list(bis.iter().map(|(a, t)| LispVal::list({
      let a = LispVal::atom(a.unwrap_or(AtomID::UNDER));
      heap.push(a.clone());
      match t {
        &Type::Bound(s) => {bvs.push(a.clone()); vec![a, LispVal::atom(self.sorts[s].atom)]}
        &Type::Reg(s, xs) => Self::deps(&bvs, vec![a, LispVal::atom(self.sorts[s].atom)], xs)
      }
    })).collect())
  }

  pub fn expr_node(&self, heap: &[LispVal], ds: &mut Option<&mut Vec<LispVal>>, e: &ExprNode) -> LispVal {
    match e {
      &ExprNode::Ref(n) => heap[n].clone(),
      &ExprNode::Dummy(a, s) => {
        let a = LispVal::atom(a);
        if let Some(ds) = ds {
          ds.push(LispVal::list(vec![a.clone(), LispVal::atom(self.sorts[s].atom)]));
        }
        a
      }
      &ExprNode::App(t, ref es) => {
        let mut args = vec![LispVal::atom(self.terms[t].atom)];
        args.extend(es.iter().map(|e| self.expr_node(heap, ds, e)));
        LispVal::list(args)
      }
    }
  }
}

#[derive(PartialEq, Eq, Hash, Debug)]
pub enum ProofHash {
  Var(usize),
  Dummy(AtomID, SortID),
  Term(TermID, Vec<usize>),
  Hyp(usize, usize),
  Thm(ThmID, Vec<usize>, usize),
  Conv(usize, usize, usize),
  Refl(usize),
  Sym(usize),
  Cong(TermID, Vec<usize>),
  Unfold(TermID, Vec<usize>, usize, usize, usize),
}

impl ProofHash {
  fn subst(de: &mut Dedup<Self>, env: &Environment,
    heap: &[ExprNode], nheap: &mut [Option<usize>], e: &ExprNode) -> usize {
    match *e {
      ExprNode::Ref(i) => nheap[i].unwrap_or_else(|| {
        let n = Self::subst(de, env, heap, nheap, &heap[i]);
        nheap[i] = Some(n);
        n
      }),
      ExprNode::Dummy(_, _) => unreachable!(),
      ExprNode::App(t, ref es) => {
        let es2 = es.iter().map(|e| Self::subst(de, env, heap, nheap, e)).collect();
        de.add_direct(ProofHash::Term(t, es2))
      }
    }
  }

  fn conv(de: &Dedup<Self>, i: usize) -> bool {
    match *de.vec[i].0 {
      ProofHash::Var(j) => j < i && Self::conv(de, j),
      ProofHash::Dummy(_, _) |
      ProofHash::Term(_, _) |
      ProofHash::Hyp(_, _) |
      ProofHash::Thm(_, _, _) |
      ProofHash::Conv(_, _, _) => false,
      ProofHash::Refl(_) |
      ProofHash::Sym(_) |
      ProofHash::Cong(_, _) |
      ProofHash::Unfold(_, _, _, _, _) => true,
    }
  }

  fn to_conv(i: usize, de: &mut Dedup<Self>) -> usize {
    if Self::conv(de, i) {i} else {
      de.add_direct(ProofHash::Refl(i))
    }
  }
}

impl NodeHash for ProofHash {
  const VAR: fn(usize) -> Self = Self::Var;
  fn from<'a>(nh: &NodeHasher<'a>, fsp: Option<&FileSpan>, r: &LispVal,
      de: &mut Dedup<Self>) -> Result<std::result::Result<Self, usize>> {
    Ok(Ok(match &**r {
      &LispKind::Atom(a) => match nh.var_map.get(&a) {
        Some(&i) => ProofHash::Var(i),
        None => match nh.lc.get_proof(a) {
          Some((_, _, p)) => return Ok(Err(de.dedup(nh, p)?)),
          None => match nh.lc.vars.get(&a) {
            Some(&(true, InferSort::Bound {sort})) => ProofHash::Dummy(a, sort),
            _ => Err(nh.err_sp(fsp, format!("variable '{}' not found", nh.fe.data[a].name)))?,
          }
        }
      },
      LispKind::MVar(_, tgt) => Err(nh.err_sp(fsp,
        format!("{}: {}", nh.fe.to(r), nh.fe.to(tgt))))?,
      LispKind::Goal(tgt) => Err(nh.err_sp(fsp, format!("|- {}", nh.fe.to(tgt))))?,
      _ => {
        let mut u = Uncons::from(r.clone());
        let head = u.next().ok_or_else(||
          nh.err_sp(fsp, format!("bad expression {}", nh.fe.to(r))))?;
        let a = head.as_atom().ok_or_else(|| nh.err(&head, "expected an atom"))?;
        let adata = &nh.fe.data[a];
        match adata.decl {
          Some(DeclKey::Term(tid)) => {
            let mut ns = Vec::new();
            for e in u { ns.push(de.dedup(nh, &e)?) }
            if ns.iter().any(|&i| Self::conv(de, i)) {
              for i in &mut ns {*i = Self::to_conv(*i, de)}
              ProofHash::Cong(tid, ns)
            } else {
              ProofHash::Term(tid, ns)
            }
          }
          Some(DeclKey::Thm(tid)) => {
            let mut ns = Vec::new();
            for e in u { ns.push(de.dedup(nh, &e)?) }
            let td = &nh.fe.thms[tid];
            let mut heap = vec![None; td.heap.len()];
            for i in 0..td.args.len() {heap[i] = Some(ns[i])}
            let rhs = Self::subst(de, &nh.fe, &td.heap, &mut heap, &td.ret);
            ProofHash::Thm(tid, ns, rhs)
          },
          None => match a {
            AtomID::CONV => match (u.next(), u.next(), u.next()) {
              (Some(tgt), Some(c), Some(p)) if u.exactly(0) =>
                ProofHash::Conv(
                  de.dedup(nh, &tgt)?,
                  Self::to_conv(de.dedup(nh, &c)?, de),
                  de.dedup(nh, &p)?),
              _ => Err(nh.err_sp(fsp, format!("incorrect :conv format {}", nh.fe.to(r))))?
            },
            AtomID::SYM => match u.next() {
              Some(p) if u.exactly(0) => ProofHash::Sym(Self::to_conv(de.dedup(nh, &p)?, de)),
              _ => Err(nh.err_sp(fsp, format!("incorrect :sym format {}", nh.fe.to(r))))?
            },
            AtomID::UNFOLD => {
              let (t, es, p) = match (u.next(), u.next(), u.next(), u.next()) {
                (Some(t), Some(es), Some(p), None) if u.exactly(0) => (t, es, p),
                (Some(t), Some(es), Some(_), Some(p)) if u.exactly(0) => (t, es, p),
                _ => Err(nh.err_sp(fsp, format!("incorrect :unfold format {}", nh.fe.to(r))))?
              };
              let tid = t.as_atom().and_then(|a| nh.fe.term(a))
                .ok_or_else(|| nh.err(&t, "expected a term"))?;
              let mut ns = Vec::new();
              for e in Uncons::from(es.clone()) { ns.push(de.dedup(nh, &e)?) }
              let lhs = de.add_direct(ProofHash::Term(tid, ns.clone()));
              let td = &nh.fe.terms[tid];
              let rhs = match &td.val {
                Some(Some(Expr {heap, head})) => {
                  let mut nheap = vec![None; heap.len()];
                  for i in 0..td.args.len() {nheap[i] = Some(ns[i])}
                  Self::subst(de, &nh.fe, heap, &mut nheap, head)
                }
                _ => return Err(nh.err(&t, "expected a definition")),
              };
              ProofHash::Unfold(tid, ns, lhs, rhs, Self::to_conv(de.dedup(nh, &p)?, de))
            },
            _ => Err(nh.err(&head, format!("term/theorem '{}' not declared", adata.name)))?
          }
        }
      }
    }))
  }
}

impl Dedup<ExprHash> {
  pub fn map_proof(&self) -> Dedup<ProofHash> {
    self.map_inj(|e| match *e {
      ExprHash::Var(i) => ProofHash::Var(i),
      ExprHash::Dummy(a, s) => ProofHash::Dummy(a, s),
      ExprHash::App(t, ref ns) => ProofHash::Term(t, ns.clone()),
    })
  }
}

impl Node for ProofNode {
  type Hash = ProofHash;
  const REF: fn(usize) -> Self = ProofNode::Ref;
  fn from(e: &Self::Hash, ids: &mut [Val<Self>]) -> Self {
    match *e {
      ProofHash::Var(i) => ProofNode::Ref(i),
      ProofHash::Dummy(a, s) => ProofNode::Dummy(a, s),
      ProofHash::Term(term, ref ns) => ProofNode::Term {
        term, args: ns.iter().map(|&i| Val::take(&mut ids[i])).collect()
      },
      ProofHash::Hyp(i, e) => ProofNode::Hyp(i, Box::new(Val::take(&mut ids[e]))),
      ProofHash::Thm(thm, ref ns, r) => ProofNode::Thm {
        thm, args: ns.iter().map(|&i| Val::take(&mut ids[i])).collect(),
        res: Box::new(Val::take(&mut ids[r]))
      },
      ProofHash::Conv(i, j, k) => ProofNode::Conv(Box::new((
        Val::take(&mut ids[i]), Val::take(&mut ids[j]), Val::take(&mut ids[k])))),
      ProofHash::Refl(i) => ProofNode::Refl(Box::new(Val::take(&mut ids[i]))),
      ProofHash::Sym(i) => ProofNode::Sym(Box::new(Val::take(&mut ids[i]))),
      ProofHash::Cong(term, ref ns) => ProofNode::Cong {
        term, args: ns.iter().map(|&i| Val::take(&mut ids[i])).collect()
      },
      ProofHash::Unfold(term, ref ns, l, r, c) => ProofNode::Unfold {
        term, args: ns.iter().map(|&i| Val::take(&mut ids[i])).collect(),
        res: Box::new((Val::take(&mut ids[l]), Val::take(&mut ids[r]), Val::take(&mut ids[c])))
      },
    }
  }
}

pub struct Subst<'a> {
  env: &'a Environment,
  heap: &'a [ExprNode],
  subst: Vec<LispVal>,
}

impl<'a> Subst<'a> {
  pub fn new(env: &'a Environment,
      heap: &'a [ExprNode], mut args: Vec<LispVal>) -> Subst<'a> {
    args.resize(heap.len(), LispVal::undef());
    Subst {env, heap, subst: args}
  }

  pub fn subst(&mut self, e: &ExprNode) -> LispVal {
    match *e {
      ExprNode::Ref(i) => {
        let e = &self.subst[i];
        if e.is_def() {return e.clone()}
        let e = self.subst(&self.heap[i]);
        self.subst[i] = e.clone();
        e
      }
      ExprNode::Dummy(_, _) => unreachable!(),
      ExprNode::App(t, ref es) => {
        let mut args = vec![LispVal::atom(self.env.terms[t].atom)];
        args.extend(es.iter().map(|e| self.subst(e)));
        LispVal::list(args)
      }
    }
  }

  pub fn subst_mut(&mut self, lc: &mut LocalContext, e: &ExprNode) -> LispVal {
    match *e {
      ExprNode::Ref(i) => {
        let e = &self.subst[i];
        if e.is_def() {return e.clone()}
        let e = self.subst_mut(lc, &self.heap[i]);
        self.subst[i] = e.clone();
        e
      }
      ExprNode::Dummy(_, s) => lc.new_mvar(InferTarget::Bound(self.env.sorts[s].atom)),
      ExprNode::App(t, ref es) => {
        let mut args = vec![LispVal::atom(self.env.terms[t].atom)];
        args.extend(es.iter().map(|e| self.subst_mut(lc, e)));
        LispVal::list(args)
      }
    }
  }
}
