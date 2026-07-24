//! Format-agnostic math model. Both front-ends target this; the emitter
//! consumes it. Constructs outside the v1 subset become `Unsupported`.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AccentKind {
    Hat,
    Bar,
    Vec,
    Tilde,
    Dot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MathNode {
    Row(Vec<MathNode>),
    Ident(String),
    Number(String),
    Op(String),
    Text(String),
    Frac(Box<MathNode>, Box<MathNode>),
    Sup(Box<MathNode>, Box<MathNode>),
    Sub(Box<MathNode>, Box<MathNode>),
    SubSup(Box<MathNode>, Box<MathNode>, Box<MathNode>),
    Sqrt(Box<MathNode>),
    Root(Box<MathNode>, Box<MathNode>),
    Fenced {
        open: String,
        close: String,
        body: Box<MathNode>,
    },
    Nary {
        op: String,
        sub: Option<Box<MathNode>>,
        sup: Option<Box<MathNode>>,
        body: Box<MathNode>,
    },
    Matrix(Vec<Vec<MathNode>>),
    Accent {
        kind: AccentKind,
        base: Box<MathNode>,
    },
    Unsupported,
}
