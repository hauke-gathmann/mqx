/** Copy during a pointer/click gesture. `navigator.clipboard` is often denied in the Tauri webview. */
export function writeClipboard(text: string): boolean {
  const field = document.createElement("textarea");
  field.value = text;
  field.setAttribute("readonly", "");
  field.style.position = "fixed";
  field.style.top = "0";
  field.style.left = "0";
  field.style.width = "1px";
  field.style.height = "1px";
  field.style.opacity = "0";
  document.body.appendChild(field);
  field.focus();
  field.select();
  let ok = false;
  try {
    ok = document.execCommand("copy");
  } catch {
    ok = false;
  }
  field.remove();
  if (ok) {
    return true;
  }
  if (navigator.clipboard?.writeText) {
    void navigator.clipboard.writeText(text);
    return true;
  }
  return false;
}
