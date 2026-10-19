use directedtype::component::{Component, Context, WasmComponent};
use directedtype::compiler::Value;
use directedtype::interaction::{Event, EventKind, MouseButton, Point};
use std::collections::HashMap;
use wasmi::{Engine, Linker, Module, Store};

#[test]
fn test_wasmi_basic_init() {
    let engine = Engine::default();
    let wat = r#"
        (module
            (func (export "add") (param i32 i32) (result i32)
                local.get 0
                local.get 1
                i32.add
            )
        )
    "#;
    let module = Module::new(&engine, wat.as_bytes()).expect("module compiles");
    let linker = Linker::new(&engine);
    let mut store = Store::new(&engine, ());
    let instance = linker.instantiate_and_start(&mut store, &module).expect("instance ok");
    let add = instance.get_typed_func::<(i32, i32), i32>(&store, "add").expect("add func ok");
    let res = add.call(&mut store, (10, 25)).expect("call ok");
    assert_eq!(res, 35);
}

#[test]
fn test_wasm_counter_component_lifecycle() {
    let wasm_bytes = std::fs::read("examples/Counter.wasm")
        .or_else(|_| std::fs::read("target/wasm32-unknown-unknown/release/examples/Counter.wasm"))
        .expect("Counter.wasm exists");
    let mut wasm_comp = WasmComponent::from_bytes(&wasm_bytes).expect("instantiate wasm ok");

    // Test on_mount with initial port 5
    let mut ports = HashMap::new();
    ports.insert("initial".to_string(), Value::Number(5.0));
    let layout = directedtype::compiler::layout::ResolvedLayout::default();
    let mut ctx = Context::new(
        directedtype::compiler::expanded::NodeId(1),
        None,
        None,
        &ports,
        &layout,
    );
    wasm_comp.on_mount(&mut ctx);

    // Dispatch "increment" -> 5 + 1 = 6
    let mut event = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::default(),
        Point::default(),
        Default::default(),
        directedtype::compiler::expanded::NodeId(1),
    );
    wasm_comp
        .dispatch("increment", &mut event, &mut ctx)
        .expect("dispatch increment ok");
    let mutations = ctx.take_mutations();
    assert_eq!(mutations, vec![("count".to_string(), Value::Number(6.0))]);

    // Dispatch "decrement" -> 6 - 1 = 5
    wasm_comp
        .dispatch("decrement", &mut event, &mut ctx)
        .expect("dispatch decrement ok");
    let mutations = ctx.take_mutations();
    assert_eq!(mutations, vec![("count".to_string(), Value::Number(5.0))]);

    // Dispatch "reset" -> 0
    wasm_comp
        .dispatch("reset", &mut event, &mut ctx)
        .expect("dispatch reset ok");
    let mutations = ctx.take_mutations();
    assert_eq!(mutations, vec![("count".to_string(), Value::Number(0.0))]);
}

#[test]
fn test_wasm_auto_discovery_and_layout_compilation() {
    let dt_path = std::path::Path::new("examples/Counter.dt");
    assert!(dt_path.exists());

    let resolver = directedtype::compiler::FsResolver;
    let mut registry = directedtype::component::ComponentRegistry::standard();
    registry.auto_discover_wasm(dt_path, &resolver);

    assert!(registry.has_component("Counter"));
    assert_eq!(
        registry.get_companion_path("Counter"),
        Some("examples/Counter.wasm")
    );

    let source = std::fs::read_to_string(dt_path).expect("read Counter.dt");
    let doc = directedtype::parser::parse_document(&source).expect("parse Counter.dt");
    let base_dir = dt_path.parent().unwrap();

    let compiled = directedtype::compiler::CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        base_dir,
        &resolver,
        &registry,
    )
    .expect("compile Counter.dt with wasm registry");

    let layout = compiled.layout();
    assert!(!layout.nodes.is_empty());
}

#[test]
fn test_wasm_audio_mixer_lifecycle_and_callback() {
    let wasm_bytes = std::fs::read("examples/slider_demo.wasm")
        .or_else(|_| std::fs::read("target/wasm32-unknown-unknown/release/examples/slider_demo.wasm"))
        .expect("slider_demo.wasm exists");
    let mut wasm_comp = WasmComponent::from_bytes(&wasm_bytes).expect("instantiate wasm ok");

    // Mount AudioMixer with initial master=75, bass=40, treble=60
    let mut ports = HashMap::new();
    ports.insert("master".to_string(), Value::Number(75.0));
    ports.insert("bass".to_string(), Value::Number(40.0));
    ports.insert("treble".to_string(), Value::Number(60.0));

    let layout = directedtype::compiler::layout::ResolvedLayout::default();
    let mut ctx = Context::new(
        directedtype::compiler::expanded::NodeId(1),
        None,
        None,
        &ports,
        &layout,
    );
    wasm_comp.on_mount(&mut ctx);

    // Dispatch "set_master" with payload 88.4 (rounds to 88)
    let mut event = Event::new(
        EventKind::Callback,
        Point::default(),
        Point::default(),
        Default::default(),
        directedtype::compiler::expanded::NodeId(1),
    );
    event.payload = Some(Value::Number(88.4));
    wasm_comp
        .dispatch("set_master", &mut event, &mut ctx)
        .expect("dispatch set_master ok");
    let mutations = ctx.take_mutations();
    assert_eq!(mutations, vec![("master".to_string(), Value::Number(88.0))]);

    // Dispatch "set_bass" with payload 22.0
    event.payload = Some(Value::Number(22.0));
    wasm_comp
        .dispatch("set_bass", &mut event, &mut ctx)
        .expect("dispatch set_bass ok");
    let mutations = ctx.take_mutations();
    assert_eq!(mutations, vec![("bass".to_string(), Value::Number(22.0))]);

    // Test auto-discovery of slider_demo.wasm for AudioMixer component
    let dt_path = std::path::Path::new("examples/slider_demo.dt");
    let resolver = directedtype::compiler::FsResolver;
    let mut registry = directedtype::component::ComponentRegistry::standard();
    let source = std::fs::read_to_string(dt_path).expect("read slider_demo.dt");
    let doc = directedtype::parser::parse_document(&source).expect("parse slider_demo.dt");

    registry.auto_discover_wasm(dt_path, &resolver);
    registry.auto_discover_wasm_for_document(&doc, dt_path, &resolver);

    assert!(registry.has_component("AudioMixer"));
}
