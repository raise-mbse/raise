import { useRaiseStore } from '../../../store/useRaiseStore';

export default function RightPanel_Squad() {
  const squad = useRaiseStore((state) => state.squad);
  const status = useRaiseStore((state) => state.workflowStatus);

  // Valeurs par défaut si Rust n'a pas encore poussé l'état de la Squad
  const agents = squad ? squad.agents : ['Architect_Agent', 'Compliance_Agent', 'Physics_Agent'];
  const squadStatus = squad ? squad.status : (status === 'Running' ? 'active' : 'idle');

  return (
    <div className="w-72 bg-slate-900/80 backdrop-blur-md border border-slate-700/50 rounded-xl p-5 shadow-2xl flex flex-col gap-4 pointer-events-auto">
      <div className="flex justify-between items-center border-b border-slate-700/50 pb-3">
        <h2 className="text-cyan-400 font-mono text-sm tracking-widest font-bold">SQUAD STATUS</h2>
        <div className={`text-xs font-mono px-2 py-1 rounded ${squadStatus === 'active' ? 'bg-cyan-500/20 text-cyan-400' : 'bg-slate-800 text-slate-400'}`}>
          {squadStatus.toUpperCase()}
        </div>
      </div>

      <div className="flex flex-col gap-3">
        {agents.map((agent, idx) => (
          <div key={idx} className="flex items-center justify-between bg-slate-950/50 p-3 rounded-lg border border-slate-800">
            <span className="text-slate-300 text-sm font-mono truncate">{agent}</span>
            <span className="relative flex h-3 w-3">
              {squadStatus === 'active' && (
                <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-cyan-400 opacity-75"></span>
              )}
              <span className={`relative inline-flex rounded-full h-3 w-3 ${squadStatus === 'active' ? 'bg-cyan-500' : 'bg-slate-600'}`}></span>
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}