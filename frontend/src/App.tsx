import React from 'react';
import { TopNav } from './components/layout/TopNav';
import { OverviewPage } from './pages/OverviewPage';
import { ModelsPage } from './pages/ModelsPage';
import { APIKeysPage } from './pages/APIKeysPage';
import { FallbackPage } from './pages/FallbackPage';
import './styles/globals.css';

const App: React.FC = () => {
  return (
    <>
      <TopNav />
      <main className="container">
        <OverviewPage />
        <ModelsPage />
        <APIKeysPage />
        <FallbackPage />
      </main>
    </>
  );
};

export default App;