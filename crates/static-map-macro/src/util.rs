use proc_macro2::Span;
use quote::{quote, ToTokens};
use syn::{parse, Generics, ItemImpl, WhereClause};

/// Extension trait for `ItemImpl` (impl block).
pub trait ItemImplExt {
    /// Instead of
    ///
    /// ```rust,ignore
    /// let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    ///
    /// let item: Item = Quote::new(def_site::<Span>())
    ///     .quote_with(smart_quote!(
    /// Vars {
    /// Type: type_name,
    /// impl_generics,
    /// ty_generics,
    /// where_clause,
    /// },
    /// {
    /// impl impl_generics ::swc_common::AstNode for Type ty_generics
    /// where_clause {}
    /// }
    /// )).parse();
    /// ```
    ///
    /// You can use this like
    ///
    /// ```rust,ignore
    // let item = Quote::new(def_site::<Span>())
    ///     .quote_with(smart_quote!(Vars { Type: type_name }, {
    ///         impl ::swc_common::AstNode for Type {}
    ///     }))
    ///     .parse::<ItemImpl>()
    ///     .with_generics(input.generics);
    /// ```
    fn with_generics(self, generics: Generics) -> Self;
}

impl ItemImplExt for ItemImpl {
    fn with_generics(mut self, mut generics: Generics) -> Self {
        // TODO: Check conflicting name

        let need_new_punct = !generics.params.empty_or_trailing();
        if need_new_punct {
            generics
                .params
                .push_punct(syn::token::Comma(Span::call_site()));
        }

        // Respan
        if let Some(t) = generics.lt_token {
            self.generics.lt_token = Some(t)
        }
        if let Some(t) = generics.gt_token {
            self.generics.gt_token = Some(t)
        }

        let ty = self.self_ty;

        // Handle generics defined on struct, enum, or union.
        let mut item: ItemImpl = {
            let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
            let item = if let Some((ref polarity, ref path, ref for_token)) = self.trait_ {
                quote! {
                    impl #impl_generics #polarity #path #for_token #ty #ty_generics #where_clause {}
                }
            } else {
                quote! {
                    impl #impl_generics #ty #ty_generics #where_clause {}

                }
            };
            parse(item.into_token_stream().into())
                .unwrap_or_else(|err| panic!("with_generics failed: {}", err))
        };

        // Handle generics added by proc-macro.
        item.generics
            .params
            .extend(self.generics.params.into_pairs());
        match self.generics.where_clause {
            Some(WhereClause {
                ref mut predicates, ..
            }) => {
                println!("FOO");
                predicates.extend(
                    generics
                        .where_clause
                        .into_iter()
                        .flat_map(|wc| wc.predicates.into_pairs()),
                )
            }
            ref mut opt @ None => *opt = generics.where_clause,
        }

        ItemImpl {
            attrs: self.attrs,
            defaultness: self.defaultness,
            unsafety: self.unsafety,
            impl_token: self.impl_token,
            brace_token: self.brace_token,
            items: self.items,
            ..item
        }
    }
}
