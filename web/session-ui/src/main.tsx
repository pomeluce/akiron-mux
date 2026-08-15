import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '@xterm/xterm/css/xterm.css';
import '@/styles/tailwind.css';
import '@/styles/app.css';
import { App } from '@/app/app';
import { installDesktopInteractionGuards } from '@/features/desktop/desktop-shell';

installDesktopInteractionGuards();

createRoot(document.querySelector('#root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
