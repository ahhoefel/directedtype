use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, FnArg, ImplItem, ItemImpl, Pat, ReturnType};

/// Procedural attribute macro for DirectedType companion components.
///
/// Annotate an `impl MyComponent` block with `#[component]`.
/// Any `pub fn <name>(&mut self, ...)` method defined in the block will automatically
/// be registered into an auto-generated `impl Component for MyComponent` with dynamic
/// event dispatching, eliminating the need to write manual string matching or boilerplate error handling.
#[proc_macro_attribute]
pub fn component(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr_str = attr.to_string();
    let is_guest = attr_str.contains("guest");
    expand_component(item, is_guest)
}

/// Procedural attribute macro specifically for `directedtype-guest` WebAssembly components.
#[proc_macro_attribute]
pub fn guest_component(_attr: TokenStream, item: TokenStream) -> TokenStream {
    expand_component(item, true)
}

fn expand_component(item: TokenStream, is_guest: bool) -> TokenStream {
    let input = parse_macro_input!(item as ItemImpl);
    let self_ty = &input.self_ty;

    let (trait_path, ctx_path, error_path, from_payload_path, event_path) = if is_guest {
        (
            quote! { directedtype_guest::Component },
            quote! { directedtype_guest::Context<'_> },
            quote! { directedtype_guest::DispatchError },
            quote! { directedtype_guest::FromPayload },
            quote! { directedtype_guest::Event },
        )
    } else {
        (
            quote! { directedtype::component::Component },
            quote! { directedtype::component::Context<'_> },
            quote! { directedtype::component::DispatchError },
            quote! { directedtype::component::FromPayload },
            quote! { directedtype::interaction::Event },
        )
    };

    let mut match_arms = Vec::new();
    let mut on_mount_impl = None;

    for item in &input.items {
        if let ImplItem::Fn(method) = item {
            // Only process public methods
            if !matches!(method.vis, syn::Visibility::Public(_)) {
                continue;
            }

            let method_name = &method.sig.ident;
            let method_str = method_name.to_string();

            // Check receiver: must take &mut self
            let takes_mut_self = method.sig.inputs.iter().any(|arg| match arg {
                FnArg::Receiver(r) => r.mutability.is_some(),
                _ => false,
            });

            if !takes_mut_self {
                continue;
            }

            // Check if it's on_mount
            if method_str == "on_mount" {
                on_mount_impl = Some(quote! {
                    fn on_mount(&mut self, ctx: &mut #ctx_path) {
                        self.on_mount(ctx);
                    }
                });
                continue;
            }

            // Process non-receiver arguments
            let other_args: Vec<_> = method
                .sig
                .inputs
                .iter()
                .filter(|arg| !matches!(arg, FnArg::Receiver(_)))
                .collect();

            let mut extract_stmts = Vec::new();
            let mut call_args = Vec::new();

            for (i, arg) in other_args.iter().enumerate() {
                match classify_arg(arg) {
                    ArgKind::Event => {
                        call_args.push(quote! { event });
                    }
                    ArgKind::Context => {
                        call_args.push(quote! { ctx });
                    }
                    ArgKind::Payload => {
                        let var_name = quote::format_ident!("__dt_arg_{}", i);
                        extract_stmts.push(quote! {
                            let #var_name = match #from_payload_path::from_payload(event.payload.as_ref()) {
                                Some(v) => v,
                                None => return Err(#error_path::Custom(
                                    format!("Invalid or missing payload for method '{}'", #method_str)
                                )),
                            };
                        });
                        call_args.push(quote! { #var_name });
                    }
                }
            }

            let call_expr = quote! { self.#method_name(#(#call_args),*) };

            let arm_body = match &method.sig.output {
                ReturnType::Default => quote! {
                    {
                        #(#extract_stmts)*
                        #call_expr;
                        Ok(())
                    }
                },
                ReturnType::Type(_, _) => quote! {
                    {
                        #(#extract_stmts)*
                        #call_expr
                    }
                },
            };

            // Support both "foo" and "on_foo" aliases automatically
            let alias_str = if let Some(base) = method_str.strip_prefix("on_") {
                base.to_string()
            } else {
                format!("on_{}", method_str)
            };

            match_arms.push(quote! {
                #method_str | #alias_str => #arm_body,
            });
        }
    }

    let on_mount_tokens = on_mount_impl.unwrap_or_else(|| quote! {});

    let expanded = quote! {
        #input

        impl #trait_path for #self_ty {
            #on_mount_tokens

            fn dispatch(
                &mut self,
                method: &str,
                event: &mut #event_path,
                ctx: &mut #ctx_path,
            ) -> Result<(), #error_path> {
                match method {
                    #(#match_arms)*
                    _ => Err(#error_path::MethodNotFound {
                        component: stringify!(#self_ty).into(),
                        method: method.into(),
                    }),
                }
            }
        }
    };

    TokenStream::from(expanded)
}

enum ArgKind {
    Event,
    Context,
    Payload,
}

fn classify_arg(arg: &FnArg) -> ArgKind {
    match arg {
        FnArg::Typed(pat_type) => {
            let ident_name = if let Pat::Ident(pat_ident) = &*pat_type.pat {
                pat_ident.ident.to_string()
            } else {
                String::new()
            };
            let type_str = quote!(#pat_type).to_string();

            if type_str.contains("Event") || ident_name == "event" {
                ArgKind::Event
            } else if type_str.contains("Context") || ident_name == "ctx" {
                ArgKind::Context
            } else {
                ArgKind::Payload
            }
        }
        FnArg::Receiver(_) => unreachable!(),
    }
}
