import {
  createSlot
} from "./chunk-A3JMCUOA.js";
import {
  require_react_dom
} from "./chunk-QQKO4ZQR.js";
import {
  require_jsx_runtime
} from "./chunk-LD7CC62Z.js";
import {
  require_react
} from "./chunk-GJSZFCZH.js";
import {
  __toESM
} from "./chunk-4MBMRILA.js";

// ../../node_modules/.pnpm/@radix-ui+react-primitive@2.1.4_@types+react-dom@18.3.7_@types+react@18.3.26__@types+re_3a81d6b806706f763d871930a34b4af6/node_modules/@radix-ui/react-primitive/dist/index.mjs
var React = __toESM(require_react(), 1);
var ReactDOM = __toESM(require_react_dom(), 1);
var import_jsx_runtime = __toESM(require_jsx_runtime(), 1);
var NODES = [
  "a",
  "button",
  "div",
  "form",
  "h2",
  "h3",
  "img",
  "input",
  "label",
  "li",
  "nav",
  "ol",
  "p",
  "select",
  "span",
  "svg",
  "ul"
];
var Primitive = NODES.reduce((primitive, node) => {
  const Slot = createSlot(`Primitive.${node}`);
  const Node = React.forwardRef((props, forwardedRef) => {
    const { asChild, ...primitiveProps } = props;
    const Comp = asChild ? Slot : node;
    if (typeof window !== "undefined") {
      window[Symbol.for("radix-ui")] = true;
    }
    return (0, import_jsx_runtime.jsx)(Comp, { ...primitiveProps, ref: forwardedRef });
  });
  Node.displayName = `Primitive.${node}`;
  return { ...primitive, [node]: Node };
}, {});

export {
  Primitive
};
//# sourceMappingURL=chunk-SM6KUIG5.js.map
