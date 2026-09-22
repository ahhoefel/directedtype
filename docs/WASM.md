Alright, I've implemented the basics of this. They're available at https://github.com/ahhoefel/directedtype if you want to see it. I'm interested in how I should connect it to a programming language for manipulating the dom and interacting with events. A critical part of a language like this is that both markup and code need to be easily distributed and safely run by the client in their viewer. That clearly suggests an interpreted language, but I'd love to stay away from javascript. I'd prefer to take another language like go or rust, compile it down to something wasm or bytecode or even binary and run in a VM or on baremetal. I'm not sure if that's realistic because I want to make sure it only has access to the dom and not other system resources. 

-----

It is entirely realistic, and you have perfectly described the exact use case for **Standalone WebAssembly (WASM)**.

Because WASM was designed to run untrusted code in the browser, it is a default-deny, hardware-isolated sandbox. If you embed a WASM runtime in your viewer application, the guest code cannot access the file system, network, or system processes unless you explicitly hand it the capabilities to do so.

Since you are writing the Directed Type engine in Rust, utilizing Rust to compile your guest scripts down to WASM creates a completely unified, highly performant stack.

Here is how you architecture this securely without ever touching JavaScript.

### 1. The Embedded VM (Wasmtime)

Instead of building your own VM or relying on V8, you embed **Wasmtime** (or Wasmer) directly into your Rust viewer. Wasmtime is a standalone WASM runtime built by the Bytecode Alliance. It compiles WASM to optimized machine code just-in-time (JIT) or ahead-of-time (AOT).

* **The Security Model:** When you load a `.wasm` file into Wasmtime, it spins up an isolated linear memory instance.
* **The Sandbox:** Do NOT enable WASI (WebAssembly System Interface) in Wasmtime. Without WASI, the guest module literally does not have the assembly instructions to ask the OS for a file or a network socket. It is completely blind and deaf to the outside world.

### 2. The Guest Language (Rust to WASM)

You author your interactive UI scripts in Rust, compiling them with the `wasm32-unknown-unknown` target. This strips out the standard library's OS bindings (since there is no OS) and yields a tiny, pure-logic binary.

Your guest script is just a library that exports an event loop or event handlers:

```rust
// In the guest WASM script

#[no_mangle]
pub extern "C" fn on_click(node_id: u32) {
    if node_id == SUBMIT_BUTTON {
        // We need to tell the host to update the graph.
        // host_set_width is a function provided by the viewer.
        unsafe { host_set_width(MODAL_ID, 500.0) };
    }
}

// Declare the host function we expect the viewer to provide
extern "C" {
    fn host_set_width(node_id: u32, width: f32);
}

```

### 3. The Bridge (Host Functions)

To allow the WASM script to manipulate your Directed Type graph, your Rust viewer explicitly maps specific internal functions into the WASM instance upon initialization. This is the *only* API the guest has.

```rust
// In the Host Viewer (using Wasmtime)

// Define the function that mutates your DAG
let set_width_func = Func::wrap(&mut store, |node_id: u32, width: f32| {
    my_dag.get_node_mut(node_id).set_width(width);
});

// Inject it into the WASM instance
let instance = Instance::new(&mut store, &module, &[set_width_func.into()])?;

```

### 4. The Performance Trap: Boundary Crossing

The only bottleneck in this architecture is the boundary crossing. Calling a host function from WASM takes a few nanoseconds. If a designer writes a script that mutates 10,000 nodes in a loop, crossing the WASM/Host boundary 10,000 times will stutter the frame rate.

**The Solution: Batched Mutations**
Instead of the guest script calling `host_set_value` for every single property, you implement a batching architecture:

1. **Shared Memory:** The host viewer allocates a block of memory (an array of bytes) inside the WASM linear memory and hands the WASM module a pointer to it.
2. **Guest Writes:** When `on_click` fires, the WASM script doesn't call host functions. It simply writes an array of mutation instructions (e.g., `[NODE_ID, PROPERTY_ENUM, VALUE]`) directly into that shared memory block.
3. **Host Reads:** The WASM function returns. The host viewer instantly reads that memory block, parses the list of mutations, and applies them all to the DAG simultaneously.
4. **DAG Resolves:** The DAG re-evaluates the topological math in one pass and hands the new layout to Vello.

This means you only cross the WASM boundary exactly twice per event (Host calls Guest `on_event` -> Guest returns to Host), regardless of how complex the script's layout mutations are.