use quote::{quote, ToTokens};
use syn::{spanned::Spanned, Attribute, Expr, Ident, Lit, Meta};

#[proc_macro_derive(ShrinkToFit, attributes(shrink_to_fit))]
pub fn derive_shrink_to_fit(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input: syn::DeriveInput = syn::parse_macro_input!(input as syn::DeriveInput);
    let type_attr: TypeAttr = TypeAttr::parse(&input.attrs);

    let crate_name = type_attr
        .crate_name
        .as_ref()
        .map(|q| q.to_token_stream())
        .unwrap_or_else(|| quote!(shrink_to_fit));

    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let body_impl = match &input.data {
        syn::Data::Struct(s) => {
            let (field_bindings, body_code) = expand_fields(&type_attr, &s.fields);

            quote!(
                match self {
                    Self { #field_bindings } => {
                        #body_code
                    }
                }
            )
        }

        syn::Data::Enum(e) => {
            let mut arms = proc_macro2::TokenStream::new();

            for v in e.variants.iter() {
                let variant_name = &v.ident;

                let (field_bindings, body_code) = expand_fields(&type_attr, &v.fields);

                arms.extend(quote!(
                    Self::#variant_name { #field_bindings } => {
                        #body_code
                    },
                ));
            }

            quote!(
                match self {
                    #arms
                }
            )
        }

        syn::Data::Union(_) => {
            panic!("union is not supported");
        }
    };

    quote! {
        impl<#impl_generics> #crate_name::ShrinkToFit for #name<#ty_generics> #where_clause {
            fn shrink_to_fit(&mut self) {
                #body_impl
            }
        }
    }
    .into()
}

#[derive(Default)]
struct TypeAttr {
    crate_name: Option<syn::Path>,
}
impl TypeAttr {
    fn parse(attrs: &[Attribute]) -> TypeAttr {
        let mut data_attr = TypeAttr::default();

        for attr in attrs {
            if attr.path().is_ident("shrink_to_fit") {
                if let Meta::List(meta) = &attr.meta {
                    let tokens = meta.tokens.clone();
                    let kv = syn::parse2::<syn::MetaNameValue>(tokens).unwrap();

                    if kv.path.is_ident("crate") {
                        if let Expr::Lit(syn::ExprLit {
                            lit: Lit::Str(s), ..
                        }) = &kv.value
                        {
                            let path = syn::parse_str::<syn::Path>(&s.value()).unwrap();
                            data_attr.crate_name = Some(path);
                        }
                    }
                }
            }
        }

        data_attr
    }
}

/// Returns `(field_bindings, body_code)`
fn expand_fields(
    type_attr: &TypeAttr,
    fields: &syn::Fields,
) -> (proc_macro2::TokenStream, proc_macro2::TokenStream) {
    let crate_name = type_attr
        .crate_name
        .as_ref()
        .map(|q| q.to_token_stream())
        .unwrap_or_else(|| quote!(shrink_to_fit));

    let mut field_bindings = proc_macro2::TokenStream::new();
    let mut body_impl = proc_macro2::TokenStream::new();

    match fields {
        syn::Fields::Named(fields) => {
            for field in fields.named.iter() {
                let field_name = field.ident.as_ref().unwrap();

                field_bindings.extend(quote!(
                    ref mut #field_name,
                ));

                body_impl.extend(quote!(
                    #crate_name::helpers::ShrinkToFitDerefSpecialization::new(#field_name).shrink_to_fit();
                ));
            }
        }

        syn::Fields::Unnamed(fields) => {
            for (i, field) in fields.unnamed.iter().enumerate() {
                let field_name = Ident::new(&format!("_{}", i), field.span());

                body_impl.extend(quote!(
                    #crate_name::helpers::ShrinkToFitDerefSpecialization::new(#field_name).shrink_to_fit();
                ));

                let index = syn::Index::from(i);
                field_bindings.extend(quote!(
                    #index: ref mut #field_name,
                ));
            }
        }

        syn::Fields::Unit => {}
    }

    (field_bindings, body_impl)
}
