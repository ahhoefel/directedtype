# DirectedType Tutorial

DirectedType is a markup language for displaying documents and applications.

Its main goals are:
* To provide high quality typographic layout and complete control for designers,
* To encourage engineering practices that support readability and reusability,
* To connect naturally to application logic to build responsive user interfaces.

This is done through a simple computational model for describing layout. Much like 
HTML, DirectedType describes a DOM -- a tree of components with various properties.
Unlike HTML, all of these properties are related through a DAG (directed acyclic graph)
of relationships. Each property is defined using expressions in terms of other properties,
constants, or application provided values. The values of these properties are then computed
by a single pass over the DAG. The resulting values are used to lay out the document.

Here's how you display "Hello world!":

```dt render
\Text(
    x: 16px, 
    y: 16px, 
    color: #000000,
    font: \Font(family: "Arial", size: 12pt)) { 
    Hello world!
}
```

```dt render hide-code padding=16px
\use "components/HCenterUp.dt"
\use "components/Card.dt"
\use "components/CardStyle.dt"
\use "components/Arrow.dt"
\env font = \Font(family: "Arial", size: 12pt);

\let card_style = \CardStyle(
    bg: #f8fafc,
    border_color: #cbd5e1,
    border_width: 1px,
    radius: 12px,
    padding_x: 24px,
    padding_y: 20px
);

\let start = \HCenterUp(y: 0px) {
    \CardFit(style: card_style, gap: 10px) {
        \Text(color: #000000) { 
            window
        }
    }
}

\let end = \HCenterUp(y: 64px) {
    \CardFit(style: card_style, gap: 10px) {
        \Text(color: #000000) { 
            \\Text
        }
    }
}

\Arrow(
    x1: (start.left + start.right)/2,
    y1: start.bottom,
    x2: (start.left + start.right)/2,
    y2: end.top
)
```

Here `\Text` is a built-in component for displaying text. It takes a list of properties as parameters: `x`, `y`, `color` and `font`.
It also has another input which is its block. In this case, its block contains the text "Hello world!".

Every time you want to make a component, you start with a slash before the component name. Component names typically start with a capital letter.
In the example above `\Font` is another component. Some components won't have parameters and the parentheses can be omitted.
Some don't have a block, like the `\Font` component.

The expression for creating a component is called a "component literal". It places the component into the DOM -- the component tree of the document.

Let's build a custom component to help us position our `\Text` components. Components are defined with the `\Component <name>(<parameters) { <body> }` command. Below we define the `\LeftMargin` component which takes a `margin` parameter of type `Distance`. What it does is places its children into the DOM and passes `margin` to their `x` value.

Below the definition of `\LeftMargin` we create a component literal of `\LeftMargin` and place two `\Text` components in its block.

```dt render 
\Component LeftMargin(margin: Distance) {
    \Children(x: margin)
}

\LeftMargin(margin: 16px) {
    \Text(
        y: 16px,
        color: #000000,
        font: \Font(family: "Arial", size: 12pt)) { 
        First line.
    }
    \Text(
        y: 36px,
        color: #000000,
        font: \Font(family: "Arial", size: 12pt)) { 
        Second line.
    }
}
```

This gives us the following DOM:

```dt render hide-code padding=16px
\use "components/HCenterUp.dt"
\use "components/HStack.dt"
\use "components/Card.dt"
\use "components/CardStyle.dt"
\use "components/Arrow.dt"
\env font = \Font(family: "Arial", size: 12pt);

\let card_style = \CardStyle(
    bg: #f8fafc,
    border_color: #cbd5e1,
    border_width: 1px,
    radius: 12px,
    padding_x: 24px,
    padding_y: 20px
);

\let start = \HCenterUp(y: 0px) {
    \CardFit(style: card_style, gap: 10px) {
        \Text(color: #000000) { 
            window
        }
    }
}

\HCenterUp(y: 64px) {
    \let lmargin = \CardFit(style: card_style, gap: 10px) {
        \Text(color: #000000) { 
            \\LeftMargin
        }
    }
}

\HCenterUp(y: 128px) {
    \HStack(gap: 32px) {
    \let textLeft = \CardFit(style: card_style, gap: 10px) {
        \Text(color: #000000) { 
            \\Text
        }
    }
    \let textRight = \CardFit(style: card_style, gap: 10px) {
        \Text(color: #000000) { 
            \\Text
        }
    }
    }
}

\Arrow(
    x1: (start.left + start.right)/2,
    y1: start.bottom,
    x2: (start.left + start.right)/2,
    y2: lmargin.top
)

\Arrow(
    x1: lmargin.left + lmargin.width/4,
    y1: lmargin.bottom,
    x2: (textLeft.left + textLeft.right)/2,
    y2: textLeft.top
)

\Arrow(
    x1: lmargin.left + 3*lmargin.width/4,
    y1: lmargin.bottom,
    x2: (textRight.left + textRight.right)/2,
    y2: textRight.top
)
```

Notice how we've removed the `x` parameter from the `\Text` components in the example above. This is because `\LeftMargin` passes an `x` value to its children. Parameters in DirectedType that don't have a default value are required, but they can be provided implicitly by the surrounding component.

This example also shows us a DAG with two edges:

```dt render hide-code padding=16px
\use "components/HCenterUp.dt"
\use "components/HStack.dt"
\use "components/ComponentPill.dt"
\use "components/Arrow.dt"
\env font = \Font(family: "Arial", size: 12pt);

\HCenterUp(y: 0px) {
    \let lmargin = \ComponentPill {
        \Part{ \\LeftMargin }
        \let margin = \Part(color: #dbeafe) { margin }
    }
}

\HCenterUp(y: 64px) {
    \HStack(gap: 32px) {
        \ComponentPill {
            \Part { \\Text }
            \let leftX = \Part(color: #dbeafe) { x }
        }
        \let textRight = \ComponentPill {
            \Part { \\Text }
            \let rightX = \Part(color: #dbeafe){ x }
        }
    }
}

\Arrow(
    x1: margin.left + margin.width/2,
    y1: margin.bottom,
    x2: (leftX.left + leftX.right)/2,
    y2: leftX.top
)

\Arrow(
    x1: margin.left + margin.width/2,
    y1: margin.bottom,
    x2: (rightX.left + rightX.right)/2,
    y2: rightX.top
)
```

The vertices in our DAG are the properties on our various component instances and there is an edge for every property definition in terms of another property.

## Properties

### Types

Each property has a type associated with it. DirectedType supports a number of basic types like `Distance`, `Float`, `Integer`, `String`, `Boolean` and `Color`. See the language documentation for how to format constants of each type. 

We've already seen the `Distance` type in the definition of the `margin` property and we've passed it the value `16px`. That is 16 logical pixels. Much like in CSS, pixels are device independent and may be represented with one or more pixels on your screen. Other units for distance include `pt` for points. Each point is 1/72nd of an inch and we use a DPI property (dots per inch) on the top level window to convert these to logical pixels. Finally, distances can be specified in `em` units which depend implicitly on the current font. One `em` is equal to the font size -- approximately the height of a capital M.

### Kinds of Properties

The properties that appear in the definition of a Component are not the only
properties that they have. There are also **private**, **alias**, **environmental**, and **state** properties.

#### Private Properties

Private properties can be declared in any block -- i.e. in the body of a component definition, the block of a component literal, or in the top scope
of a DirectedType file.

They are declared with the `\let` command.

```dt
\let <name> = <expression>;
```

They are accessible anywhere in the scope that they are defined and their order of declaration doesn't matter. Remember, properties are assigned expressions that are only evaluated after the entire DAG is constructed.

```dt compile-error
\Component Foo(x: Distance) {
    \let y = 30px;
    \Text(x, y, font: \Font(family: "Arial", size: 12pt)){
        Hello World!
    }
}

\let f = \Foo(x: 20px)
\let sum = f.x + f.y; // Breaks because f.y is not visible
```
In the example above, we make a private property `y` on the `Foo` component. It can be passed to `\Text`. In this example we're using a property shorthand. Really `\Text(x,y)` is expanded to `\Text(x: x, y: y)` where the first `x` and first `y` are the public properties on `\Text` and the second `x` is the public property on `\Foo` while the second `y` is the private property on `Foo`.

Notice in the example that we can also make a private property that refers to a component literal: `\let f = \Foo(x: 20px)`. In these cases we don't need the trailing semicolon. Also, the component literal is also placed into the DOM at this point, so `f` simply becomes a reference to an existing component in the DOM. It does not store a component outside of the DOM for later use. This can be really useful for naming components and extracting their public properties. However, we can not extract private properties from a component in this way.

#### Alias Properties

Aliases are public properties of a component but differ from the public properties of a component in that they cannot be set when declaring the component.

They are declared as follows:

```dt
\alias <name> = <expression>;
```

If we change the private property `y` in our previous example to be
an alias, it's visible outside the component definition.

```dt compiles
\Component Foo(x: Distance) {
    \alias y = 30px;
    \Text(x, y, font: \Font(family: "Arial", size: 12pt)){
        Hello World!
    }
}

\let f = \Foo(x: 20px)
\let sum = f.x + f.y; // Works! y is a visible property on Foo.
```

We often depend on components having certain visible properties.
For example, we may want to depend on a property's width or height.
Aliases are used to make sure that these properties are all available
when one depends on the other.

```dt compiles
\Component Rectangle(area: Area, width: Distance) {
    \alias height = area / width;
}

\Component Rectangle(area: Area, height: Distance) {
    \alias width = area / height;
}
```

This example brings up another feature: DirectedType supports component
overloading. This is when multiple component definitions share the
same component name. For this to be possible, they need to have distinct required parameters (i.e parameters without defaults).

#### Environmental Properties

Wiring together properties can be tedious. Environmental properties
are automatically passed into labeled environmental parameters with the same name. They only exist within the lexical scope in which they are declared.

Here's the declaration syntax:
```dt
\env <name> = <expression>;
```

To mark a parameter as an environmental property, we simply prefix
it with `env`:

```dt
\Component Foo(env background: Color) {
    ...
}
```

For instance, the `font` parameter on the `\Text` component is
an environmental parameter. So, if we declare an environmental property called `font` it will automatically be passed into any `\Text` in the same scope.

```dt render
\env font = \Font(family: "Arial", size: 12pt) 
\Text(x: 16px, y: 16px, color: #000000) { Hello world! }
```

Components are designed to be hermetic and modular. Environmental properties do not affect components in the internal lexical scope (i.e. the body) of a component definition by default. The following will fail to compile:

```dt compile-error
\Component Foo() {
    \Text(x: 16px, y: 16px, color: #000000) { 
        Hello world!
    }
}
\env font = \Font(family: "Arial", size: 12pt) 
\Foo() // The font does not reach the Text.
```

If you want an environmental variable to apply to
the internals of a component, you must make it
an environmental parameter.

```dt render
\Component Foo(env font: Font) {
    \Text(x: 16px, y: 16px, color: #000000) { 
        Hello world!
    }
}
\env font = \Font(family: "Arial", size: 12pt) 
\Foo()
```

On the other hand, environmental properties do automatically get passed to components in the
external lexical scope of a component. That is,
they'll get passed to components in the block
of a component literal:

```dt render
\Component Foo() {
    \Children
}
\env font = \Font(family: "Arial", size: 12pt) 
\Foo {
    \Text(x: 16px, y: 16px, color: #000000) { 
        Hello world!
    }
}
```

This works because `font` and `\Text` are in the same lexical scope. We can prevent this by capturing the environmental property and shadowing it with a private non-environmental property.

```dt compile-error
\Component Foo(env font: Font) {
    \let font; // font is no longer passed to children.
    \Children
}
\env font = \Font(family: "Arial", size: 12pt) 
\Foo {
    \Text(x: 16px, y: 16px, color: #000000) { 
        Hello world!
    }
}
```

Finally, it's very useful for a component to define its environmental properties locally, knowing they'll affect all components in both its internal and external lexical scopes.

```dt render
\Component Arial {
    \env font = \Font(family: "Arial", size: 12pt) 
    \Text(x: 16px, y: 16px, color: #000000){
        Internal lexical scope
    }
    \Children
}

\Arial {
    \Text(x: 16px, y: 36px, color: #000000) { 
        External lexical scope
    }
}
```

#### State Properties

Our final kind of property is a **state** property. Much like aliases, these are public properties that can't be set by parameters. Unlike aliases, they are not set by a DirectedType expression, but rather by some external application logic.

Applications have two ways of altering DirectedType. They can either edit the DOM itself, or they can provide input values to the DAG by using state properties.

```dt
\state <name>: <type> = <expression>;
```

Often private and alias properties don't need to include their types when they're defined since those types can often be inferred from their expressions. State properties need to declare their type so that application code can be strictly typed.

We'll see examples of setting state when we discuss how to make application logic and reactive user interfaces.

## Navigating the DOM

### Pulling Properties Down

We've already seen examples where components set parameters on their children.
Children can also pull properties from their parents. In the following example,
we'll use the built-in `\Rect` component which draws rectangles.

```dt render
\Component Foo(x: Distance, width: Distance) {
    \Rect(x, y: x, width, height: width, color: #A00000)
    \Children
}

\Foo(x: 16px, width: 64px) {
    \Rect(
        x: parent.x + 16px, 
        y: parent.x + 16px,
        width: parent.width,
        height: parent.width,
        color: #00FF0080
    )
}
```

When `parent` keyword appears in a component, it refers to the component above it in the DOM. If we use `parent` in a component definition, it refers to the enclosing
component wherever that component is instantiated. In the next example, when we define `resolved_x`, the expression `parent.x` refers to the `x` property on the enclosing red `\Rect`. Let's examine the definition of the green `\Rect`'s `width` as `parent.parent.width`. Since that definition occurs in the green `\Rect` in the `\Foo` component definition, `parent` refers to `\Foo` and `parent.parent` refers to the enclosing component -- again the red `\Rect`.

```dt render
\Component Foo() {
    \let resolved_x = parent.x + 16px;
    \Rect(
        x: resolved_x, 
        y: resolved_x,
        width: parent.parent.width,
        height: parent.parent.width,
        color: #00FF0080
    )
}

\Rect(x: 16px, y: 16px, width: 64px, height: 64px, color: #A00000) {
    \Foo
}
```


```dt render hide-code padding=16px
\use "components/HCenterUp.dt"
\use "components/HStack.dt"
\use "components/ComponentPill.dt"
\use "components/Arrow.dt"
\env font = \Font(family: "Arial", size: 12pt);

\ComponentPill(x: 0px, y: 0px) {
    \let redRect = \Part(color: #FF9090) { \\Rect }
    \let redWidth = \Part(color: #dbeafe) { width }
}

\ComponentPill(x: (redRect.width - foo.width)/2, y: 64px) {
    \let foo = \Part { \\Foo }
}

\ComponentPill(x: 0, y: 128px) {
    \let greenRect = \Part(color: #90FF90) { \\Rect }
    \let greenWidth = \Part(color: #dbeafe){ width }
}

\let a1 = \Arrow(
    x1: redWidth.left + redWidth.width/2,
    y1: redWidth.bottom,
    x2: (greenWidth.left + greenWidth.right)/2,
    y2: greenWidth.top,
)

\let a2 = \Arrow(
    x1: greenRect.left + greenRect.width/2,
    y1: greenRect.top,
    x2: foo.left + foo.width/2,
    y2: foo.bottom,
)

\let a3 = \Arrow(
    x1: foo.left + foo.width/2,
    y1: foo.top,
    x2: redRect.left + redRect.width/2,
    y2: redRect.bottom,
)

\let t = \Text(x:a1.x1 + 8px, y: (a1.y1 + a1.y2)/2 - t.height/2, color: #000000, font: \Font(family: "Arial", size: 10pt)){parent.parent.width}

\let p = \Text(x:a2.x1 + 8px, y: (a2.y1 + a2.y2)/2 - p.height/2, color: #000000, font: \Font(family: "Arial", size: 10pt)){parent}

\let pp = \Text(x:a3.x1 + 8px, y: (a3.y1 + a3.y2)/2 - pp.height/2, color: #000000, font: \Font(family: "Arial", size: 10pt)){parent}
```

### Pulling Properties Up

There are two kinds of children that can be placed on a component: those that are found in its internal lexical scope (i.e. in the component definition), and those that come from its external lexical scope (i.e. in the component literal's block) and are placed using the `\Children` command.

Children in the internal lexical scope can be given labels using the `\let` command and we can expose derived properties on the defined component using `\alias`.

In the following example, the `\PaddedRect` derives public properties for `x`, `y`, `width`, `height` from the rectangle `r`. It doesn't create any other visual components that allow you to see the padding, but other components can read those public properties.

```dt render
\Component PaddedRect() {
    \alias x = r.x - 16px;
    \alias y = r.x - 16px;
    \alias width = r.width * 2;
    \alias height = r.height * 2;

    \let r = \Rect(
        x: 32px, 
        y: 32px,
        width: 64px,
        height: 64px,
        color: #00AA00
    )
}
\PaddedRect
```

Let's now make an outline component that pulls properties up from a child placed in its external lexical scope.

```dt render
# \Component PaddedRect() {
#     \alias x = r.x - 16px;
#     \alias y = r.x - 16px;
#     \alias width = r.width + 32px;
#     \alias height = r.height + 32px;
# 
#     \let r = \Rect(
#         x: 32px, 
#         y: 32px,
#         width: 64px,
#         height: 64px,
#         color: #00AA00
#     )
# }
\Component Outline() {
    \assert children.len == 1 as "Should only have one child."
    \Rect(
        x: only(children.x), 
        y: only(children.y), 
        width: only(children.width),
        height: only(children.height),
        color: #00000000,
        border_width: 1px,
        border_color: #000000,
    )
    \Children
}

\Outline { \PaddedRect }
```

In the example above, `children` refers collectively to all the children coming from the external lexical scope (i.e. those passed into the component literal's block). You can only use `children` with aggregation functions on its properties. The `only` aggregation function is particular in that it gives a compile time error if there's not exactly one child. Other aggregation functions include `max`, `min` and `sum`.

```dt render
\Component Outline() {
    \let x = min(children.x);
    \let y = min(children.y);
    \let x2 = max(children.right);
    \let y2 = max(children.bottom);
    \Children
    \Rect(x, y, width: x2-x, height: y2-y,
        color: #00000000,
        border_width: 1px,
        border_color: #000000,
    )
}

\Outline { 
    \Rect(x: 16px, y:32px, width: 32px, height: 32px, color: #FF0000)
    \Rect(x: 48px, y:16px, width: 32px, height: 32px, color: #00FF00)
    \Rect(x: 80px, y:48px, width: 32px, height: 32px, color: #0000FF)
 }
```

### Passing Properties to Siblings

Finally, children can refer to their siblings. We can use this to build
lists with the help of recursion. Within the `\Children` command, you can
make expressions using `prev` and `next`.


```dt render
\Component NumberRect(x: Distance, n: Integer, color: Color ) {
    \Rect(x: x, y: x, width: 16px, height: 16px, color)
    \let t = \Text(
        x: x + (16px - t.width)/2,
        y: x + (16px - t.height)/2,
        font: \Font(family: "Arial", size: 12pt),
        color: #000,
        text: n.to_fixed()
    )
}

\Component Diagonal {
    \Children(
        x: prev ? prev.x + 16px: 16px, 
        n: prev ? prev.n + 1 : 1,
        color: next ? #F88 : #8F8
    )
}

\Diagonal{
    \NumberRect
    \NumberRect
    \NumberRect
    \NumberRect
}
```

In the previous example, both `x` and `n` are set recursively on each
of the subsequent children of `\Diagonal`. The color is also set to green
on the last child.

