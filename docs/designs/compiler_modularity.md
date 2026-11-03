# DirectedType Compiler Architecture: Modularity, Single-Source Type/Eval Tables, & Method Standardization

- **Status:** Planned
- **Topic:** Eliminating Cross-Cutting Compiler Duplication, Standardizing Primitive Method Syntax, Unifying Type Inference, and Splitting Monolithic Compiler Modules
- **Target Area:** `src/compiler/expand/typecheck.rs`, `src/compiler/expand/rewrite.rs`, `src/compiler/expand/component.rs`, `src/compiler/eval.rs`, `src/compiler/value.rs`, `src/ast.rs`, `directedtype-guest`

---

## 1. Motivation & Problem Statement

In a well-factored compiler, most natural language features (adding a built-in method, adding a component, or even adding a dimensional scalar type like `Duration`) should require changes in **at most 2–4 small, focused modules**.

Currently, adding or modifying a primitive type or built-in operation requires touching **12–15 files** and reading **5,000+ lines** across four oversized files:
- [`src/compiler/expand/typecheck.rs`](file:///Users/hoefel/dev/directedtype/src/compiler/expand/typecheck.rs) (~1,765 lines)
- [`src/compiler/expand/component.rs`](file:///Users/hoefel/dev/directedtype/src/compiler/expand/component.rs) (~1,735 lines)
- [`src/compiler/eval.rs`](file:///Users/hoefel/dev/directedtype/src/compiler/eval.rs) (~1,595 lines)
- [`src/compiler/expand/rewrite.rs`](file:///Users/hoefel/dev/directedtype/src/compiler/expand/rewrite.rs) (~965 lines)

This document specifies five concrete refactoring phases so that future changes—by humans or AI agents—require reading and editing only the directly relevant 100–300 line module.

---

## 2. Phase 1: Standardize on Method Call Syntax for Primitive Operations

### Problem
Currently, primitive conversions and formatting operations are supported as **both** free-standing function calls (`to_px(d)`, `to_seconds(t)`, `to_fixed(x, 2)`) **and** method calls (`d.to_px()`, `t.to_seconds()`, `x.to_fixed(2)`).
- In the language, having two ways to write the same conversion creates inconsistency.
- In the compiler, every conversion is implemented **4 times**:
  1. `Expr::Call` in `typecheck.rs` (twice: `infer_expr_literal_for_typecheck` and `infer_expr_type`)
  2. `Expr::MethodCall` in `typecheck.rs` (twice)
  3. `Expr::Call` in `eval.rs`
  4. `Expr::MethodCall` in `eval.rs`

### Design Decision: Methods on Primitives vs. Free Functions

1. **Primitive Conversions & Formatting $\rightarrow$ Strictly Method Calls (`receiver.method(...)`):**
   Single-receiver type conversions and formatting operations on primitive types (`Distance`, `Duration`, `Integer`, `Float`) must use **method call syntax**:
   - **Distance:**
     - `d.to_px() -> Float`
     - `d.to_pt() -> Float`
     - `d.to_float(unit)` (replaces `distance_to_float(d, unit)`, e.g. `d.to_float(em)`, `d.to_float(px)`, `d.to_float(pt)`)
   - **Duration:**
     - `t.to_seconds() -> Float`
     - `t.to_ms() -> Float`
   - **Integer / Float:**
     - `n.to_float() -> Float` (remove `to_number` alias; standardize on `to_float`)
     - `n.to_integer() -> Integer`
   - **Numeric Formatting (`Integer`, `Float`, `Distance`, `Area`, `Duration`):**
     - `n.to_fixed(decimals: Integer) -> String`
     - `n.to_precision(sigfigs: Integer) -> String`
   - **Path / String:**
     - `p.append(segment: String) -> String`

   *(Note: If the compiler internally lowers `0` for `Integer` type annotations via a synthetic call, keep an internal-only `__to_integer(0)` or eliminate that synthetic call once Phase 2 unifies `StaticType` zero-coercion.)*

2. **Aggregates, Multi-Argument Math, & Constructors $\rightarrow$ Free Functions (`fn(...)`):**
   Operations that aggregate collections, take multiple symmetric arguments, or construct values remain free functions:
   - **Aggregates / Child Queries:** `min(...)`, `max(...)`, `sum(...)`, `only(...)`, `clamp(val, min, max)`
   - **Math Functions:** `abs(x)`, `round(x)`, `floor(x)`, `ceil(x)`, `sqrt(x)`, `sin(x)`, `cos(x)`, `tan(x)`, `atan2(y, x)`, `hypot(dx, dy)`
   - **Color Constructors:** `rgb(r, g, b)`, `rgba(r, g, b, a)`, `hsl(h, s, l)`, `hsla(h, s, l, a)`
   - **Internal Compiler Intrinsics:** `text_width(...)`, `text_height(...)`, `text_baseline_offset(...)`, `span_metric(...)`, `font_cap_height(...)`, `font_x_height(...)`, `font_ascent(...)`, `font_descent(...)`, `font_line_gap(...)`, `font_line_height(...)`

### Implementation Steps
1. Update any `.dt` files in `components/`, `examples/`, and `tests/` that call `to_px(x)`, `to_pt(x)`, `distance_to_float(x, u)`, `to_seconds(t)`, `to_ms(t)`, `to_float(x)`, `to_number(x)`, `to_integer(x)`, `to_fixed(x, n)`, or `to_precision(x, n)` as free functions to use method syntax (`x.to_px()`, `x.to_float(u)`, `t.to_seconds()`, etc.).
2. Remove the free-function arms for `to_px`, `to_pt`, `distance_to_float`, `to_seconds`, `to_ms`, `to_float`, `to_number`, `to_integer`, `to_fixed`, and `to_precision` from `Expr::Call` in `src/compiler/expand/typecheck.rs` and `src/compiler/eval.rs`.
3. Add a helpful diagnostic in `typecheck.rs` if a user calls one of these as a free function (e.g., `"'to_seconds' is a method on Duration; use 'expr.to_seconds()' instead of 'to_seconds(expr)'"`).

---

## 3. Phase 2: Unify Type Inference on `StaticType` & Centralize Zero-Polymorphism

### Problem 1: Dual Type Inference in `typecheck.rs`
[`src/compiler/expand/typecheck.rs`](file:///Users/hoefel/dev/directedtype/src/compiler/expand/typecheck.rs) currently contains two parallel type inference functions:
- [`infer_expr_type(expr, ctx) -> Result<StaticType, CompileError>`](file:///Users/hoefel/dev/directedtype/src/compiler/expand/typecheck.rs#L1122): The real static typechecker returning `StaticType`.
- [`infer_expr_literal_for_typecheck(expr, ctx) -> Option<Literal>`](file:///Users/hoefel/dev/directedtype/src/compiler/expand/typecheck.rs#L483): A legacy helper that fabricates dummy `Literal` values (such as `Literal::Distance(0.0, DistanceUnit::Px, span)` or `Literal::Duration(0, DurationUnit::S, span)`) solely to test `lit.matches_type_name(...)` or infer ternary branch types.

Because both functions traverse `Expr::Binary`, `Expr::Unary`, `Expr::Call`, and `Expr::MethodCall`, every operator and built-in is written twice in `typecheck.rs`.

### Problem 2: Zero-Polymorphism Scattered Across 6 Locations
Coercing the integer literal `0` (`StaticType::ZeroInteger`) to `Distance(0px)`, `Area(0px²)`, `Duration(0s)`, or `Float(0.0)` is manually repeated in:
1. `coerce_typed_binding_expr` (`src/compiler/expand/typecheck.rs`)
2. `validate_and_coerce_primitive_port_literal` (`src/compiler/expand/typecheck.rs`)
3. `rewrite_expr` ternary branches (`src/compiler/expand/rewrite.rs`, twice)
4. `default_expr_for_state_with_enums` (`src/compiler/expand/rewrite.rs`, twice)
5. `eval_expr` binary ops, ternary branches, and aggregate functions (`src/compiler/eval.rs`)
6. `CompiledDocument::set_state` (`src/compiler/compiled.rs`)

### Implementation Steps
1. **Add Zero-Coercion Helpers to `StaticType` and `Value`:**
   - On `StaticType`:
     ```rust
     impl StaticType {
         /// If `expr` is `Literal::Integer(0, span)` and `self` is a type that coerces from `0`
         /// (`Distance`, `Area`, `Duration`, `Float`), returns the coerced `Expr::Literal`.
         pub fn coerce_zero_expr(&self, expr: Expr) -> Expr { ... }

         /// Returns the default zero/empty `Literal` for this `StaticType` (used by uninitialized `\state`).
         pub fn default_literal(&self, span: Span, enums: &HashMap<String, EnumDef>) -> Expr { ... }
     }
     ```
   - On `Value`:
     ```rust
     impl Value {
         /// Coerces `Value::Integer(0)` or `Value::Integer(i)` / `Value::Float(f)` to `target_type`.
         pub fn coerce_to_type_name(self, target_type: &str) -> Value { ... }
     }
     ```
2. **Delete `infer_expr_literal_for_typecheck` and `Literal::matches_type_name`:**
   - Replace all call sites of `infer_expr_literal_for_typecheck(expr, ctx)` in `typecheck.rs` and `rewrite.rs` with `infer_expr_type(expr, ctx).ok()`.
   - In `validate_and_coerce_primitive_port_literal`, check `inferred_type.matches(&expected_static_type)` and call `expected_static_type.coerce_zero_expr(...)`.
   - In `rewrite_expr` (`Expr::Ternary`), use `infer_expr_type(&other_rewritten, ctx)` and call `other_ty.coerce_zero_expr(chosen)`.

---

## 4. Phase 3: Simplify Compile-Time Ternary Specialization in `expand/`

### Problem
During component expansion (`src/compiler/expand/rewrite.rs` lines 338–427 and `src/compiler/expand/component.rs` lines 1504–1540), the compiler attempts to constant-fold ternary conditions (`cond ? then_expr : else_expr`) before building the `VariableGraph`. To do so, it maintains:
- `resolve_ident_or_member_to_literal` (`src/compiler/expand/typecheck.rs` lines 725–930), which re-implements `==`, `!=`, `&&`, and `||` over `Literal`s.
- Duplicate `match lit { Literal::Integer(...) => Value::Integer(...), ... }` conversion tables in both `rewrite.rs` and `component.rs`.

### Why Does Compile-Time Ternary Specialization Exist?
There are only **two** places where static ternary pruning is actually required before runtime DAG evaluation:
1. **Event Handler Ports (`is_event_port == true`):** Event handler bindings (`click: enabled ? self.on_click : unset`) are wired into the component event dispatch table at compile time rather than evaluated as DAG equations.
2. **Preventing Phantom Cycles When Conditioning on Static Configuration Ports:** When a component uses a constant boolean/enum port or `children.len == 0` to choose between two different dependency directions (e.g., `has_children ? max(children.right) : 0`), folding the known-constant condition at expansion time avoids adding the dead branch's edges to `VariableGraph`.

### Implementation Steps
1. **Single `Literal::to_value()` Method (`src/compiler/value.rs`):**
   - Implement `Value::from_literal_const(lit: &Literal) -> Option<Value>` in `src/compiler/value.rs` for all unit-independent literals (`Integer`, `Float`, `Distance(_, Px, _)`, `Area(_, Px, _)`, `Duration`, `String`, `Bool`, `Color`, `Enum`).
   - Delete the duplicate `match lit { ... }` blocks in `rewrite.rs` and `component.rs`.
2. **Delegate Constant Condition Evaluation to `eval_expr`:**
   - Instead of maintaining `resolve_ident_or_member_to_literal`'s custom `BinaryOp::{Eq, Ne, And, Or}` evaluator over `Literal`s in `typecheck.rs`, populate `const_env: HashMap<VarId, Value>` once per component expansion using a shared helper `build_component_const_env(...)` and evaluate `eval_expr(&rewritten_cond, &const_env)`.
   - Only keep `is_handler_expr_unset` for `unset` handler comparisons (`handler == unset`).

---

## 5. Phase 4: Single-Source Operator & Built-in Registry (`src/compiler/builtins.rs` & `src/compiler/operators.rs`)

### Problem
Currently, the type signature and runtime behavior of every binary operator (`+`, `-`, `*`, `/`, `%`, `<`, `==`), built-in function (`min`, `max`, `sum`, `clamp`, `abs`, `round`, `hypot`, `rgb`, `hsl`), and primitive method (`.to_px()`, `.to_seconds()`, `.to_fixed()`) are split across giant `match` blocks in `typecheck.rs` and `eval.rs`.

### Implementation Steps
1. **Create `src/compiler/operators.rs` (~250 lines):**
   - Co-locate binary and unary operator type inference (`typecheck_binary_op(op, left_ty, right_ty, span) -> Result<StaticType, CompileError>`) and runtime evaluation (`eval_binary_op(op, left_val, right_val, span) -> Result<Value, CompileError>`) in one self-contained module.
   - Because `typecheck_binary_op` and `eval_binary_op` sit side-by-side in the same 250-line file, adding or modifying an operator rule on a type (like `Duration` or `Area`) requires reading and editing only `src/compiler/operators.rs`.
2. **Create `src/compiler/builtins.rs` (~300 lines):**
   - Co-locate primitive method signatures + evaluation (`typecheck_method_call` and `eval_method_call`) and pure math/aggregate function signatures + evaluation (`typecheck_builtin_call` and `eval_builtin_call`).
3. **Create `src/compiler/text_intrinsics.rs` (~300 lines):**
   - Move the internal text measurement intrinsics (`text_width`, `text_height`, `text_baseline_offset`, `span_metric`, `font_cap_height`, `font_x_height`, `font_ascent`, `font_descent`, `font_line_gap`, `font_line_height`) out of `eval.rs` (where they currently occupy lines 879–1175, nearly 300 lines of `eval.rs`!) into `src/compiler/text_intrinsics.rs`.

---

## 6. Phase 5: Split Oversized Expansion Modules by Responsibility

### Target Module Layout
After Phases 1–4, the compiler's module sizes will drop dramatically:

| File | Current Size | Target Size | Responsibility |
| :--- | :--- | :--- | :--- |
| `src/compiler/operators.rs` | *(in `typecheck.rs` & `eval.rs`)* | ~250 lines | Binary & unary operator type rules and evaluation |
| `src/compiler/builtins.rs` | *(in `typecheck.rs` & `eval.rs`)* | ~300 lines | Primitive methods (`.to_px()`, `.to_seconds()`, etc.) and math/aggregate functions |
| `src/compiler/text_intrinsics.rs` | *(in `eval.rs`)* | ~300 lines | Parley text/span/font measurement intrinsics |
| `src/compiler/eval.rs` | 1,595 lines | ~350 lines | Graph topological evaluation, incremental invalidation, and `Expr` dispatch |
| `src/compiler/expand/typecheck.rs` | 1,764 lines | ~600 lines | `StaticType`, port validation, and AST expression type inference |
| `src/compiler/expand/rewrite.rs` | 967 lines | ~500 lines | Lexical/member/env scoping and unit lowering (`pt`, `em`) |

With this layout, an agent or developer working on:
- **A new built-in method or math function** only reads `src/compiler/builtins.rs` (~300 lines).
- **A dimensional operator rule** only reads `src/compiler/operators.rs` (~250 lines).
- **Text/font layout metrics** only reads `src/compiler/text_intrinsics.rs` (~300 lines).

