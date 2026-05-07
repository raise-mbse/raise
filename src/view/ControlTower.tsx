import WorldModelCanvas from './layers/canvas/WorldModelCanvas';
import LeftPanel_Mandate from './layers/overlay/LeftPanel_Mandate';
import RightPanel_Squad from './layers/overlay/RightPanel_Squad';
import BottomPanel_Hitl from './layers/overlay/BottomPanel_Hitl';

export default function ControlTower() {
  return (
    <div className="relative h-screen w-screen overflow-hidden bg-slate-950">
      
      {/* LAYER Z-0 : La 3D (Boîte Transparente) */}
      <div className="absolute inset-0 z-0 pointer-events-auto">
        <WorldModelCanvas />
      </div>
      
      {/* LAYER Z-10 : L'UI (HUD) */}
      {/* On désactive les pointer-events sur le conteneur pour pouvoir cliquer sur la 3D à travers,
          et on les réactive (pointer-events-auto) à l'intérieur des panneaux. */}
      <div className="absolute inset-0 z-10 pointer-events-none p-6 flex flex-col justify-between">
        
        {/* Ligne du Haut : Titre et Statut */}
        <div className="flex justify-between items-start">
          <div className="text-slate-400 text-xs font-mono tracking-widest bg-slate-900/50 p-2 rounded">
            RAISE // CONDORCET CONTINUUM
          </div>
          <div className="text-emerald-400 text-xs font-mono animate-pulse bg-emerald-900/30 p-2 rounded border border-emerald-800">
            [SYS_ONLINE]
          </div>
        </div>

        {/* Ligne Centrale : Les Panneaux Latéraux */}
        <div className="flex justify-between items-center flex-1 my-6">
          <LeftPanel_Mandate />
          <RightPanel_Squad />
        </div>

        {/* Le Tiroir HITL (Position absolue gérée dans le composant) */}
        <BottomPanel_Hitl />

      </div>
    </div>
  );
}