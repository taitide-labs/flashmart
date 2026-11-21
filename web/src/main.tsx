import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { createFetchTransport, FlashMartClient } from './api/client';
import { StubTransport } from './api/stubTransport';
import { App } from './App';
import { createFlashMartStore } from './stores/flashmartStore';
import './styles/tokens.css';
import './styles/base.css';
import './ui/ui.css';

const useLiveApi = import.meta.env.VITE_API_MODE === 'live';
const stubTransport = useLiveApi ? null : new StubTransport();
const transport = stubTransport ?? createFetchTransport();
const client = new FlashMartClient(transport);
const store = createFlashMartStore({ client });

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App store={store} stubTransport={stubTransport} />
  </StrictMode>,
);
