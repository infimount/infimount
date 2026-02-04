"use client";
import {
  Primitive
} from "./chunk-SM6KUIG5.js";
import "./chunk-A3JMCUOA.js";
import "./chunk-QGVOWARK.js";
import "./chunk-QQKO4ZQR.js";
import {
  require_jsx_runtime
} from "./chunk-LD7CC62Z.js";
import {
  require_react
} from "./chunk-GJSZFCZH.js";
import {
  __toESM
} from "./chunk-4MBMRILA.js";

// ../../node_modules/.pnpm/@radix-ui+react-label@2.1.8_@types+react-dom@18.3.7_@types+react@18.3.26__@types+react@_cd6cc0b3e0f63b58f1138c67dd48c07f/node_modules/@radix-ui/react-label/dist/index.mjs
var React = __toESM(require_react(), 1);
var import_jsx_runtime = __toESM(require_jsx_runtime(), 1);
var NAME = "Label";
var Label = React.forwardRef((props, forwardedRef) => {
  return (0, import_jsx_runtime.jsx)(
    Primitive.label,
    {
      ...props,
      ref: forwardedRef,
      onMouseDown: (event) => {
        var _a;
        const target = event.target;
        if (target.closest("button, input, select, textarea")) return;
        (_a = props.onMouseDown) == null ? void 0 : _a.call(props, event);
        if (!event.defaultPrevented && event.detail > 1) event.preventDefault();
      }
    }
  );
});
Label.displayName = NAME;
var Root = Label;
export {
  Label,
  Root
};
//# sourceMappingURL=@radix-ui_react-label.js.map
