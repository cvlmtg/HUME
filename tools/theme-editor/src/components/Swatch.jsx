export default function Swatch({ color, size }) {
  return (
    <span style={{
      display: "inline-block", width: size || 10, height: size || 10, borderRadius: 2,
      background: color || "transparent", border: "1px solid rgba(255,255,255,0.15)", flexShrink: 0,
    }} />
  );
}
