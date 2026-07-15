import './polyfills.ts'; // must stay first — establishes the Buffer global before the SDK loads

import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App.tsx';
import '@solana/wallet-adapter-react-ui/styles.css';
import './index.css';

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
