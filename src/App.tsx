import { useEffect } from 'react';
import { useRaiseStore } from './store/useRaiseStore';
import ControlTower from './view/ControlTower';
import './styles/globals.css';

export default function App() {
  const initSystem = useRaiseStore((state) => state.initSystem);
  const isReady = useRaiseStore((state) => state.isReady);

  useEffect(() => {
    console.log('🚀 Démarrage de RAISE Control Tower...');
    initSystem();
    // Le nettoyage des listeners se ferait ici si on démontait App (rare dans Tauri)
  }, [initSystem]);

  if (!isReady) {
    return (
      <div className="flex items-center justify-center h-screen w-screen bg-slate-900 text-cyan-400">
        <p className="animate-pulse">Initialisation du lien Neuro-Symbolique...</p>
      </div>
    );
  }

  return <ControlTower />;
}