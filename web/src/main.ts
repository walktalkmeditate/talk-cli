import './style.css';

// Scaffold stub. Real wiring (terminal, REPL, ASR pipeline) lands in later
// units; for now we paint a calm rust placeholder into #screen so the build
// produces a page that renders, and dismiss the zero-JS loading element.
const RUST = 'rgb(210, 146, 118)';

function mountPlaceholder(): void {
  const screen = document.getElementById('screen');
  if (!screen) throw new Error('missing #screen mount');

  const placeholder = document.createElement('div');
  placeholder.style.position = 'absolute';
  placeholder.style.inset = '0';
  placeholder.style.display = 'flex';
  placeholder.style.alignItems = 'center';
  placeholder.style.justifyContent = 'center';
  placeholder.style.color = RUST;
  placeholder.style.fontSize = '0.95rem';
  placeholder.style.letterSpacing = '0.04em';
  placeholder.textContent = 'talk — loading…';
  screen.appendChild(placeholder);

  const loading = document.getElementById('loading');
  if (loading) {
    loading.classList.add('gone');
    setTimeout(() => loading.remove(), 600);
  }
}

mountPlaceholder();
