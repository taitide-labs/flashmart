import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { createFetchTransport, FlashMartClient } from './api/client';
import { App } from './App';
import { createFlashMartStore } from './stores/flashmartStore';
import './styles/tokens.css';
import './styles/base.css';
import './ui/ui.css';

const baseUrl = import.meta.env.VITE_API_BASE_URL ?? '';
const transport = createFetchTransport(baseUrl);
const client = new FlashMartClient(transport);
const store = createFlashMartStore({ client });

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App store={store} />
  </StrictMode>,
);
