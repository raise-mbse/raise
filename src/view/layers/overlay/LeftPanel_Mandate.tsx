import { useState } from 'react';
import { useRaiseStore } from '../../../store/useRaiseStore';

export default function LeftPanel_Mandate() {
  const [prompt, setPrompt] = useState('');
  const startMission = useRaiseStore((state) => state.startMission);
  const status = useRaiseStore((state) => state.workflowStatus);

  const handleLaunch = () => {
    if (prompt.trim() === '') return;
    // Génération d'un ID de mission unique pour l'exemple
    const missionId = `MSN-${Math.floor(Math.random() * 1000)}`;
    startMission(missionId, prompt);
    setPrompt('');
  };

  return (
    <div className="w-80 bg-slate-900/80 backdrop-blur-md border border-slate-700/50 rounded-xl p-5 shadow-2xl flex flex-col gap-4 pointer-events-auto">
      <div>
        <h2 className="text-emerald-400 font-mono text-sm tracking-widest font-bold mb-1">NOUVEAU MANDAT</h2>
        <p className="text-slate-400 text-xs">Exprimez votre intention d'architecture.</p>
      </div>

      <textarea
        value={prompt}
        onChange={(e) => setPrompt(e.target.value)}
        disabled={status === 'Running'}
        className="w-full h-32 bg-slate-950/50 border border-slate-700 rounded-lg p-3 text-sm text-slate-200 placeholder-slate-600 focus:outline-none focus:border-emerald-500 transition-colors resize-none disabled:opacity-50"
        placeholder="Ex: Adapte le système de freinage du Rover Lunaire pour supporter une masse de 500kg..."
      />

      <button
        onClick={handleLaunch}
        disabled={status === 'Running' || prompt.trim() === ''}
        className="w-full bg-emerald-600 hover:bg-emerald-500 disabled:bg-slate-700 text-white font-mono text-sm py-2 px-4 rounded-lg transition-all"
      >
        {status === 'Running' ? 'SQUAD DÉPLOYÉE...' : 'LANCER LA SQUAD'}
      </button>
    </div>
  );
}