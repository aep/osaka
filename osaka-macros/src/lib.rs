extern crate proc_macro;
#[macro_use]
extern crate syn;
#[macro_use]
extern crate quote;
extern crate proc_macro2;

use proc_macro::TokenStream;
use syn::export::ToTokens;
use syn::ItemFn;

#[proc_macro_attribute]
pub fn osaka(_args: TokenStream, input: TokenStream) -> TokenStream {
    let mut f = parse_macro_input!(input as ItemFn);

    let output = match f.decl.output {
        syn::ReturnType::Default => quote! {()},
        syn::ReturnType::Type(_, t) => quote! {#t},
    };

    f.decl.output = match syn::parse(
        (quote! {
            -> impl ::std::ops::Generator<Yield=::osaka::Again, Return=#output>
        })
        .into(),
    ) {
        Ok(v) => v,
        Err(e) => return e.to_compile_error().into(),
    };

    let oblock = f.block;
    f.block = match syn::parse(
        (quote! {{
            move||{
                #oblock
            }
        }})
        .into(),
    ) {
        Ok(v) => v,
        Err(e) => return e.to_compile_error().into(),
    };

    f.into_token_stream().into()
}
