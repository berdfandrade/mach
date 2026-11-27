use mach_codegen::{Entity, Op};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::{ToTokens, format_ident, quote};
use syn::{
    FnArg, Ident, ItemImpl, ItemStruct, ItemTrait, LitStr, Pat, Path, TraitItem, parse_macro_input,
    parse_quote,
};
fn output(result: syn::Result<Tokens>) -> TokenStream {
    result.unwrap_or_else(syn::Error::into_compile_error).into()
}
fn reject<T>(node: impl ToTokens, message: &str) -> syn::Result<T> {
    Err(syn::Error::new_spanned(node, message))
}

#[proc_macro_attribute]
pub fn entity(args: TokenStream, input: TokenStream) -> TokenStream {
    let args: Tokens = args.into();
    let attr: syn::Attribute = parse_quote!(#[entity(#args)]);
    let item = parse_macro_input!(input as ItemStruct);
    output(
        mach_codegen::table_attr(&attr)
            .and_then(|t| mach_codegen::entity(&item, t))
            .and_then(|e| expand_entity(item, e)),
    )
}
fn expand_entity(mut item: ItemStruct, e: Entity) -> syn::Result<Tokens> {
    let original = item.clone();
    if item.attrs.iter().any(|a| !a.path().is_ident("doc")) {
        return reject(
            &item,
            "entity owns serialization and row mapping; only documentation attributes are supported",
        );
    }
    let name = &item.ident;
    let vis = &item.vis;
    let input = format_ident!("{}Input", name);
    let schema = format_ident!("__mach_schema_{}", name);
    let mut inputs = Vec::new();
    let mut validations = Vec::new();
    let mut binds = Vec::new();
    let mut reads = Vec::new();
    let mut defaults = Vec::new();
    for (field, f) in item.fields.iter_mut().zip(&e.fields) {
        let ident = field.ident.as_ref().unwrap();
        let ty = &field.ty;
        let col = &f.name;
        field.attrs.retain(|a| a.path().is_ident("doc"));
        if !f.expose {
            field.attrs.push(parse_quote!(#[serde(skip_serializing)]))
        }
        reads.push(quote!(#ident: ::mach::sqlx::Row::try_get(row,#col)?));
        if !f.write {
            continue;
        }
        let default = if let Some(v) = f.default {
            let fun = format_ident!("__{}_{}_default", name.to_string().to_lowercase(), ident);
            let s = fun.to_string();
            let val = if f.optional {
                quote!(Some(#v))
            } else {
                quote!(#v)
            };
            defaults.push(quote!(fn #fun()->#ty {#val}));
            quote!(#[serde(default=#s)])
        } else {
            quote!()
        };
        inputs.push(quote!(#default pub #ident:#ty));
        binds.push(quote!(.bind(&value.#ident)));
        if f.min.is_some() || f.max.is_some() {
            let min = f.min.unwrap_or(0);
            let max = f.max.unwrap_or(usize::MAX);
            let validate = quote!(if !(#min..=#max).contains(&text.chars().count()){return Err(::mach::db::Error::Invalid)});
            validations.push(if f.optional {
                quote!(if let Some(text)=&self.#ident {#validate})
            } else {
                quote!({let text=&self.#ident;#validate})
            });
        }
    }
    let sql = e.crud();
    let find = sql.find;
    let list = sql.list;
    let list_desc = sql.list_desc;
    let create = sql.create;
    let delete = sql.delete;
    let table = e.table;
    let (owner, by_owner) = if e.fields.iter().any(|f| f.owner) {
        (quote!(owner:i64,), quote!(.bind(owner)))
    } else {
        (quote!(), quote!())
    };
    let update=sql.update.map(|s|quote!{
        pub async fn update<'e>(db:impl ::mach::db::PgExecutor<'e>,id:i64,#owner value:#input)->::mach::db::Result<Self>{
            ::mach::db::Validate::validate(&value)?;
            Ok(::mach::sqlx::query_as(#s)#(#binds)*.bind(id)#by_owner.fetch_one(db).await?)
        }
    });
    Ok(quote! {
        #[derive(::mach::serde::Serialize)] #[serde(crate="mach::serde")]
        #item
        impl<'r> ::mach::sqlx::FromRow<'r,::mach::sqlx::postgres::PgRow> for #name {
            fn from_row(row:&'r ::mach::sqlx::postgres::PgRow)->::std::result::Result<Self,::mach::sqlx::Error>{ Ok(Self{#(#reads),*}) }
        }
        #(#defaults)*
        #[derive(::mach::serde::Deserialize)] #[serde(crate="mach::serde",deny_unknown_fields)]
        #vis struct #input { #(#inputs),* }
        impl ::mach::db::Validate for #input {
            fn validate(&self)->::mach::db::Result<()> { #(#validations)* Ok(()) }
        }
        impl #name {
            pub const TABLE:&'static str=#table;
            pub async fn find<'e>(db:impl ::mach::db::PgExecutor<'e>,id:i64)->::mach::db::Result<Option<Self>>{
                Ok(::mach::sqlx::query_as(#find).bind(id).fetch_optional(db).await?)
            }
            pub async fn find_or_fail<'e>(db:impl ::mach::db::PgExecutor<'e>,id:i64)->::mach::db::Result<Self>{
                Self::find(db,id).await?.ok_or(::mach::db::Error::NotFound)
            }
            pub async fn create<'e>(db:impl ::mach::db::PgExecutor<'e>,#owner value:#input)->::mach::db::Result<Self>{
                ::mach::db::Validate::validate(&value)?;
                Ok(::mach::sqlx::query_as(#create)#(#binds)*#by_owner.fetch_one(db).await?)
            }
            #update
            pub async fn list<'e>(db:impl ::mach::db::PgExecutor<'e>,page: ::mach::db::Pagination)->::mach::db::Result<Vec<Self>>{
                let page=page.validate()?;
                let sql=match page.order { ::mach::db::Order::Asc=>#list, ::mach::db::Order::Desc=>#list_desc };
                Ok(::mach::sqlx::query_as(sql).bind(page.limit).bind(page.offset).fetch_all(db).await?)
            }
            pub async fn delete<'e>(db:impl ::mach::db::PgExecutor<'e>,id:i64,#owner)->::mach::db::Result<()>{
                if ::mach::sqlx::query(#delete).bind(id)#by_owner.execute(db).await?.rows_affected()==0 {return Err(::mach::db::Error::NotFound)} Ok(())
            }
        }
        #[allow(unused_macros)] macro_rules! #schema {
            ($repo:item)=>{ ::mach::__repository!{#table #original $repo} };
        }
        #[allow(unused_imports)] pub(crate) use #schema as #name;
    })
}

#[proc_macro_attribute]
pub fn repository(entity: TokenStream, input: TokenStream) -> TokenStream {
    let entity = parse_macro_input!(entity as Path);
    let item = parse_macro_input!(input as ItemTrait);
    quote!(#entity!{#item}).into()
}
struct Repo {
    table: LitStr,
    model: ItemStruct,
    item: ItemTrait,
}
impl syn::parse::Parse for Repo {
    fn parse(s: syn::parse::ParseStream) -> syn::Result<Self> {
        Ok(Self {
            table: s.parse()?,
            model: s.parse()?,
            item: s.parse()?,
        })
    }
}
#[doc(hidden)]
#[proc_macro]
pub fn __repository(input: TokenStream) -> TokenStream {
    let Repo { table, model, item } = parse_macro_input!(input as Repo);
    output((|| {
        let e = mach_codegen::entity(&model, table.value())?;
        let name = &item.ident;
        let vis = &item.vis;
        let entity = &model.ident;
        let mut methods = Vec::new();
        if !item.generics.params.is_empty()
            || !item.supertraits.is_empty()
            || item.unsafety.is_some()
            || !item.attrs.is_empty()
        {
            return reject(&item, "expected a plain query declaration");
        }
        for member in &item.items {
            let TraitItem::Fn(m) = member else {
                return reject(member, "expected a query method");
            };
            let sig = &m.sig;
            if sig.asyncness.is_none()
                || m.default.is_some()
                || !sig.generics.params.is_empty()
                || sig.unsafety.is_some()
                || sig.abi.is_some()
                || !m.attrs.is_empty()
            {
                return reject(m, "expected an async query signature");
            }
            let q = mach_codegen::derived(&e, &sig.ident.to_string())
                .map_err(|s| syn::Error::new(sig.ident.span(), s))?;
            if q.params.len() != sig.inputs.len() {
                return reject(sig, "incorrect query parameter count");
            }
            let mut binds = Vec::new();
            for (a, f) in sig.inputs.iter().zip(&q.params) {
                let FnArg::Typed(arg) = a else {
                    return reject(a, "expected a named parameter");
                };
                let Pat::Ident(p) = arg.pat.as_ref() else {
                    return reject(a, "expected a named parameter");
                };
                let actual = arg.ty.to_token_stream().to_string().replace(' ', "");
                if actual != f.kind.rust()
                    && !(f.kind == mach_codegen::Kind::String && actual == "&str")
                {
                    return reject(&arg.ty, "parameter type does not match entity field");
                }
                let ident = &p.ident;
                binds.push(quote!(.bind(#ident)));
            }
            let result = match q.op {
                Op::Find => quote!(Option<#entity>),
                Op::Delete => quote!(u64),
                Op::Exists => quote!(bool),
                Op::Count => quote!(i64),
            };
            if sig.output.to_token_stream().to_string() != quote!(->Result<#result>).to_string() {
                return reject(&sig.output, "unsupported return type for derived query");
            }
            let sql = q.sql;
            let run = match q.op {
                Op::Find => {
                    quote!(::mach::sqlx::query_as(#sql)#(#binds)*.fetch_optional(db).await?)
                }
                Op::Delete => {
                    quote!(::mach::sqlx::query(#sql)#(#binds)*.execute(db).await?.rows_affected())
                }
                _ => quote!(::mach::sqlx::query_scalar(#sql)#(#binds)*.fetch_one(db).await?),
            };
            let args = &sig.inputs;
            let method = &sig.ident;
            methods.push(quote!(pub async fn #method<'e>(db:impl ::mach::db::PgExecutor<'e>,#args)->::mach::db::Result<#result>{Ok(#run)}));
        }
        Ok(quote!(#vis struct #name; impl #name {#(#methods)*}))
    })())
}

#[proc_macro_attribute]
pub fn controller(args: TokenStream, input: TokenStream) -> TokenStream {
    let base = parse_macro_input!(args as LitStr).value();
    let mut item = parse_macro_input!(input as ItemImpl);
    output((|| {
        if !base.starts_with('/') || base.ends_with('/') {
            return reject(&item, "expected a base path such as /users");
        }
        let mut routes = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for member in &mut item.items {
            if let syn::ImplItem::Fn(m) = member {
                for attr in std::mem::take(&mut m.attrs) {
                    let method = attr
                        .path()
                        .get_ident()
                        .map(Ident::to_string)
                        .unwrap_or_default();
                    if !matches!(method.as_str(), "get" | "post" | "put" | "patch" | "delete") {
                        m.attrs.push(attr);
                        continue;
                    }
                    let suffix = attr.parse_args::<LitStr>()?.value();
                    if !suffix.is_empty() && !suffix.starts_with('/') {
                        return reject(attr, "expected an empty path or /segment");
                    }
                    let path = mach_codegen::route_path(&base, &suffix)
                        .map_err(|msg| syn::Error::new_spanned(&attr, msg))?;
                    if !seen.insert((path.clone(), method.clone())) {
                        return reject(attr, "duplicate route");
                    }
                    let verb = format_ident!("{method}");
                    let handler = &m.sig.ident;
                    routes
                        .push(quote!(.route(#path, ::mach::axum::routing::#verb(Self::#handler))));
                }
            }
        }
        let ty = &item.self_ty;
        Ok(
            quote!(#item impl #ty { pub fn routes()->::mach::axum::Router<::mach::http::AppState>{::mach::axum::Router::new()#(#routes)*} }),
        )
    })())
}
