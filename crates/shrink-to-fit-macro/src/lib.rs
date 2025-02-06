use quote::quote;

#[proc_macro_derive(ShrinkToFit)]
pub fn derive_shrink_to_fit(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input: syn::DeriveInput = syn::parse_macro_input!(input as syn::DeriveInput);

    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let body_impl = match &input.data {
        syn::Data::Struct(s) => {}

        syn::Data::Enum(e) => {}

        syn::Data::Union(u) => {
            panic!("union is not supported: {:?}", u);
        }
    };

    quote! {
        impl<#impl_generics> ShrinkToFit for #name<#ty_generics> #where_clause {
            fn shrink_to_fit(&mut self) {
                #body_impl
            }
        }
    }
}
