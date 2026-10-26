//! Keep generated declarations separate from source names and imported Lean names.

pub(super) struct Names {
    pub namespace: String,
    pub expression: String,
    pub theorem: String,
}

impl Names {
    pub fn new(source: &str) -> Self {
        Self {
            namespace: format!("{source}_generated"),
            expression: format!("{source}_expr"),
            theorem: format!("{source}_correct"),
        }
    }

    pub fn reference(&self) -> String {
        format!("_root_.{}.{}", self.namespace, self.expression)
    }
}
