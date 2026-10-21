// Local stand-in for the Design type runtime: resolves props, {{holes}}, sc-if and sc-for so boards can be screenshotted.
document.addEventListener('DOMContentLoaded', () => {
  const script = document.querySelector('script[data-dc-script]');
  const schema = JSON.parse(script.dataset.props);
  const q = new URLSearchParams(location.search);
  const props = {};
  for (const [k, s] of Object.entries(schema)) {
    if (k === '$preview') continue;
    props[k] = q.has(k) ? (s.editor === 'boolean' ? q.get(k) === 'true' : q.get(k)) : s.default;
  }
  class DCLogic { constructor() { this.props = props; } }
  const Component = new Function('DCLogic', script.textContent + '\nreturn Component;')(DCLogic);
  const vals = new Component().renderVals();
  const get = (p, sc) => p.trim().split('.').reduce((o, k) => (o == null ? undefined : o[k]), sc);
  const interp = (s, sc) => s.replace(/\{\{\s*([^}]+?)\s*\}\}/g, (m, p) => { const v = get(p, sc); return v == null ? '' : String(v); });
  const key = (el, a) => el.getAttribute(a).replace(/^\{\{\s*|\s*\}\}$/g, '');
  function proc(node, sc) {
    for (const c of [...node.childNodes]) {
      if (c.nodeType === 3) { c.nodeValue = interp(c.nodeValue, sc); continue; }
      if (c.nodeType !== 1) continue;
      const tag = c.tagName.toLowerCase();
      if (tag === 'sc-if') {
        if (get(key(c, 'value'), sc)) { proc(c, sc); c.replaceWith(...c.childNodes); } else c.remove();
      } else if (tag === 'sc-for') {
        const list = get(key(c, 'list'), sc) || [], as = c.getAttribute('as'), out = [];
        for (const item of list) { const t = document.createElement('div'); t.innerHTML = c.innerHTML; proc(t, { ...sc, [as]: item }); out.push(...t.childNodes); }
        c.replaceWith(...out);
      } else {
        for (const a of [...c.attributes]) if (a.value.includes('{{')) c.setAttribute(a.name, interp(a.value, sc));
        if (tag === 'textarea') c.value = interp(c.value, sc);
        proc(c, sc);
      }
    }
  }
  proc(document.querySelector('x-dc'), vals);
  document.documentElement.dataset.ready = '1';
});
