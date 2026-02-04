import {
  Primitive
} from "./chunk-27VIL26U.js";
import {
  require_jsx_runtime
} from "./chunk-LD7CC62Z.js";
import {
  require_react
} from "./chunk-GJSZFCZH.js";
import {
  __toESM
} from "./chunk-4MBMRILA.js";

// ../../node_modules/.pnpm/@radix-ui+react-visually-hidden@1.2.3_@types+react-dom@18.3.7_@types+react@18.3.26__@ty_699add8356bb40b0a726133b69555ec2/node_modules/@radix-ui/react-visually-hidden/dist/index.mjs
var React = __toESM(require_react(), 1);
var import_jsx_runtime = __toESM(require_jsx_runtime(), 1);
var VISUALLY_HIDDEN_STYLES = Object.freeze({
  // See: https://github.com/twbs/bootstrap/blob/main/scss/mixins/_visually-hidden.scss
  position: "absolute",
  border: 0,
  width: 1,
  height: 1,
  padding: 0,
  margin: -1,
  overflow: "hidden",
  clip: "rect(0, 0, 0, 0)",
  whiteSpace: "nowrap",
  wordWrap: "normal"
});
var NAME = "VisuallyHidden";
var VisuallyHidden = React.forwardRef(
  (props, forwardedRef) => {
    return (0, import_jsx_runtime.jsx)(
      Primitive.span,
      {
        ...props,
        ref: forwardedRef,
        style: { ...VISUALLY_HIDDEN_STYLES, ...props.style }
      }
    );
  }
);
VisuallyHidden.displayName = NAME;
var Root = VisuallyHidden;

export {
  VISUALLY_HIDDEN_STYLES,
  VisuallyHidden,
  Root
};
//# sourceMappingURL=chunk-QDMRYARR.js.map
