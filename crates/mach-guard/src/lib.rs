use proc_macro2::Span;
use serde::Serialize;
use syn::{
    Expr, ItemFn, Stmt,
    spanned::Spanned,
    visit::{self, Visit},
};

#[derive(Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    pub file: String,
    pub line: usize,
    pub code: String,
    pub certainty: &'static str,
    pub severity: &'static str,
    pub message: String,
}
impl Finding {
    pub fn new(
        file: &str,
        line: usize,
        code: &str,
        certainty: &'static str,
        message: &str,
    ) -> Self {
        Self {
            file: file.into(),
            line,
            code: code.into(),
            certainty,
            severity: if certainty == "proven" {
                "error"
            } else {
                "warning"
            },
            message: message.into(),
        }
    }
}
pub fn analyze(path: &str, src: &str, strict: bool) -> syn::Result<Vec<Finding>> {
    let file = syn::parse_file(src)?;
    let mut scan = Scan {
        file: path,
        findings: Vec::new(),
        loops: 0,
        async_depth: 0,
    };
    if file.items.is_empty() && (path.ends_with("/service.rs") || path.ends_with("/repository.rs"))
    {
        scan.push(
            Span::call_site(),
            "ARCH004",
            "advisory",
            "optional module contains no declarations",
        );
    }
    scan.visit_file(&file);
    let mut offset = 0;
    let mut line = 1;
    for t in rustc_lexer::tokenize(src, rustc_lexer::FrontmatterAllowed::No) {
        let text = &src[offset..offset + t.len as usize];
        offset += t.len as usize;
        if matches!(
            t.kind,
            rustc_lexer::TokenKind::LineComment { doc_style: None }
                | rustc_lexer::TokenKind::BlockComment {
                    doc_style: None,
                    ..
                }
        ) {
            let body = text.trim_start_matches('/').trim_start_matches('*').trim();
            let allowed = [
                "SAFETY:",
                "SECURITY:",
                "COMPAT:",
                "WHY:",
                "SPDX-",
                "Copyright ",
                "rustfmt",
                "noinspection",
            ]
            .iter()
            .any(|prefix| body.starts_with(prefix));
            let redundant = ["return the result", "find the user"]
                .contains(&body.trim_end_matches('.').to_ascii_lowercase().as_str());
            if !allowed && (strict || redundant) {
                scan.findings.push(Finding::new(
                    path,
                    line,
                    "STYLE001",
                    if strict { "proven" } else { "heuristic" },
                    "ordinary comment; document the reason with an allowed category",
                ))
            }
        }
        line += text.bytes().filter(|c| *c == b'\n').count();
    }
    Ok(scan.findings)
}
struct Scan<'a> {
    file: &'a str,
    findings: Vec<Finding>,
    loops: usize,
    async_depth: usize,
}
impl Scan<'_> {
    fn push(&mut self, s: Span, code: &str, certainty: &'static str, msg: &str) {
        self.findings.push(Finding::new(
            self.file,
            s.start().line,
            code,
            certainty,
            msg,
        ))
    }
}
impl<'ast> Visit<'ast> for Scan<'_> {
    fn visit_expr_unsafe(&mut self, e: &'ast syn::ExprUnsafe) {
        self.push(e.span(), "SEC001", "proven", "unsafe block");
        visit::visit_expr_unsafe(self, e)
    }
    fn visit_signature(&mut self, s: &'ast syn::Signature) {
        if let Some(u) = s.unsafety {
            self.push(u.span(), "SEC001", "proven", "unsafe function")
        }
        visit::visit_signature(self, s)
    }
    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        if let Some(u) = i.unsafety {
            self.push(u.span(), "SEC001", "proven", "unsafe implementation")
        }
        visit::visit_item_impl(self, i)
    }
    fn visit_item_fn(&mut self, f: &'ast ItemFn) {
        if let [Stmt::Expr(Expr::Call(call),None)]=f.block.stmts.as_slice() && !call.args.is_empty() && f.sig.inputs.len()==call.args.len() && f.sig.inputs.iter().zip(&call.args).all(|(a,e)|matches!((a,e),(syn::FnArg::Typed(a),Expr::Path(p)) if matches!(a.pat.as_ref(),syn::Pat::Ident(i) if p.path.is_ident(&i.ident)))) {
            self.push(f.sig.ident.span(),"ARCH001","advisory","function forwards every argument unchanged")
        }
        let n = usize::from(f.sig.asyncness.is_some());
        self.async_depth += n;
        visit::visit_item_fn(self, f);
        self.async_depth -= n;
    }
    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if f.attrs.iter().any(|a|["post","patch","put"].iter().any(|n|a.path().is_ident(n)))
            && f.sig.inputs.iter().any(|arg|matches!(arg,syn::FnArg::Typed(arg) if matches!(arg.ty.as_ref(),syn::Type::Path(t) if t.path.segments.last().is_some_and(|s|s.ident=="Json")))) {
            self.push(f.sig.span(),"SEC003","advisory","JSON handler accepts a body without the validated Input extractor");
        }
        let n = usize::from(f.sig.asyncness.is_some());
        self.async_depth += n;
        visit::visit_impl_item_fn(self, f);
        self.async_depth -= n
    }
    fn visit_expr_async(&mut self, e: &'ast syn::ExprAsync) {
        self.async_depth += 1;
        visit::visit_expr_async(self, e);
        self.async_depth -= 1
    }
    fn visit_expr_call(&mut self, e: &'ast syn::ExprCall) {
        if let Expr::Path(p) = e.func.as_ref()
            && p.path.segments.last().is_some_and(|s| {
                ["query", "query_as", "query_scalar"].contains(&s.ident.to_string().as_str())
            })
            && e.args.first().is_some_and(|a| {
                let val = if let Expr::Reference(r) = a {
                    r.expr.as_ref()
                } else {
                    a
                };
                matches!(val,Expr::Macro(m) if m.mac.path.is_ident("format"))
            })
        {
            self.push(
                e.span(),
                "SEC002",
                "heuristic",
                "query text is interpolated; review parameter binding",
            );
        }
        if self.async_depth > 0
            && let Expr::Path(p) = e.func.as_ref()
        {
            let parts = p
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect::<Vec<_>>()
                .join("::");
            if parts == "std::thread::sleep" || parts.starts_with("std::fs::") {
                self.push(
                    p.span(),
                    "PERF002",
                    "heuristic",
                    "synchronous operation inside async code",
                )
            }
        }
        visit::visit_expr_call(self, e)
    }
    fn visit_expr_for_loop(&mut self, e: &'ast syn::ExprForLoop) {
        self.loops += 1;
        visit::visit_expr_for_loop(self, e);
        self.loops -= 1
    }
    fn visit_expr_method_call(&mut self, e: &'ast syn::ExprMethodCall) {
        if e.method == "fetch_all" && self.file.ends_with("/controller.rs") {
            self.push(
                e.span(),
                "PERF003",
                "advisory",
                "verify that the explicit collection query has a database-side limit",
            );
        }
        if self.loops > 0
            && ["fetch_one", "fetch_optional", "fetch_all"].contains(&e.method.to_string().as_str())
        {
            self.push(
                e.method.span(),
                "PERF004",
                "advisory",
                "database fetch inside a loop",
            )
        }
        visit::visit_expr_method_call(self, e)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_findings_have_distinct_evidence_levels() {
        let source = r#"impl Controller {
            #[post("/")] async fn collect(value:Json<Input>) {
                for id in ids { sqlx::query(&format!("select {}",id)).fetch_all(db).await; }
            }
        }"#;
        let found = analyze("src/modules/items/controller.rs", source, false).unwrap();
        for code in ["PERF003", "PERF004", "SEC002", "SEC003"] {
            assert!(
                found
                    .iter()
                    .any(|f| f.code == code && f.severity == "warning")
            );
        }
        let ordinary = analyze(
            "src/modules/items/controller.rs",
            r#"fn find(){sqlx::query("select $1").bind(id).fetch_optional(db);}"#,
            false,
        )
        .unwrap();
        assert!(ordinary.is_empty());
    }
    #[test]
    fn lexical_comments() {
        let f = analyze(
            "a.rs",
            "fn x(){ let _ = r#\"// text\"#; /* WHY: required */ // redundant\n}",
            true,
        )
        .unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].code, "STYLE001")
    }
    #[test]
    fn recognizes_raw_c_strings_and_doc_comments() {
        let source = r##"/// Docs
fn f() { let _ = cr#"quote " // literal"#; //// ordinary
}"##;
        let f = analyze("app.rs", source, true).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].line, 2);
    }
    #[test]
    fn checks_async_and_wrappers() {
        let f = analyze(
            "a.rs",
            "async fn x(){std::thread::sleep(d);} fn y(a:i32)->i32{g(a)}",
            false,
        )
        .unwrap();
        assert!(f.iter().any(|f| f.code == "ARCH001"));
        assert!(f.iter().any(|f| f.code == "PERF002"))
    }
}
