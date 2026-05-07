import { motion, AnimatePresence } from 'framer-motion';
import { useRaiseStore } from '../../../store/useRaiseStore';

export default function BottomPanel_Hitl() {
  const hitlAlert = useRaiseStore((state) => state.hitlAlert);
  const resolveHitl = useRaiseStore((state) => state.resolveHitl);

  // Couleurs dynamiques selon la criticité
  const getRiskColors = (level: string) => {
    switch (level) {
      case 'Red': return 'border-red-500 bg-red-950/90 text-red-400';
      case 'Blue': return 'border-blue-500 bg-blue-950/90 text-blue-400';
      case 'Orange':
      default: return 'border-orange-500 bg-orange-950/90 text-orange-400';
    }
  };

  return (
    <AnimatePresence>
      {hitlAlert && (
        <motion.div
          initial={{ y: 100, opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          exit={{ y: 100, opacity: 0 }}
          transition={{ type: 'spring', damping: 20, stiffness: 100 }}
          className="absolute bottom-8 left-1/2 -translate-x-1/2 w-[600px] pointer-events-auto"
        >
          <div className={`border-2 rounded-xl p-6 shadow-[0_0_30px_rgba(0,0,0,0.5)] backdrop-blur-xl ${getRiskColors(hitlAlert.riskLevel)}`}>
            
            <div className="flex items-center gap-3 mb-4">
              <div className="animate-pulse h-4 w-4 rounded-full bg-current"></div>
              <h2 className="font-mono text-lg tracking-widest font-bold">VALIDATION HUMAINE REQUISE</h2>
            </div>
            
            <p className="text-white mb-6 text-sm">
              {hitlAlert.message || "Une règle métier experte a levé une exception. Veuillez arbitrer."}
            </p>

            <div className="flex gap-4">
              <button
                onClick={() => resolveHitl(hitlAlert.nodeId, true)}
                className="flex-1 bg-white/10 hover:bg-white/20 border border-white/20 text-white font-mono py-2 rounded-lg transition-all"
              >
                OVERRIDE (JUSTIFIER)
              </button>
              <button
                onClick={() => resolveHitl(hitlAlert.nodeId, false)}
                className="flex-1 bg-black/40 hover:bg-black/60 text-white font-mono py-2 rounded-lg transition-all"
              >
                REJETER
              </button>
            </div>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}