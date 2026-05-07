import { create } from 'zustand';
import { invokeCmd, listenEvent } from '../api/tauri';
import { SpatialGraph, ExecutionStatus, HitlAlert, Squad, WorkflowView } from '../types/bindings';
import { UnlistenFn } from '@tauri-apps/api/event';

interface RaiseState {
  // --- ÉTATS ---
  isReady: boolean;
  spatialGraph: SpatialGraph | null;
  workflowStatus: ExecutionStatus;
  hitlAlert: HitlAlert | null;
  squad: Squad | null;

  // --- ACTIONS ---
  initSystem: () => Promise<void>;
  startMission: (missionId: string, prompt: string) => Promise<void>;
  resolveHitl: (nodeId: string, approved: boolean) => Promise<void>;
}

// Variables pour stocker les fonctions de nettoyage (Unlisten)
let unlistenTopology: UnlistenFn | null = null;
let unlistenHitl: UnlistenFn | null = null;
let unlistenSquad: UnlistenFn | null = null;

// CORRECTION ESLINT : On retire 'get' des paramètres car il n'est pas utilisé pour le moment
export const useRaiseStore = create<RaiseState>((set) => ({
  isReady: false,
  spatialGraph: null,
  workflowStatus: 'Idle',
  hitlAlert: null,
  squad: null,

  initSystem: async () => {
    try {
      const initialGraph = await invokeCmd<SpatialGraph>('get_spatial_topology');
      set({ spatialGraph: initialGraph, isReady: true });
    } catch (error) {
      // CORRECTION ESLINT : On utilise 'error' dans le console.warn
      console.warn("Topologie initiale non disponible ou vide :", error);
      set({ isReady: true }); 
    }

    if (!unlistenTopology) {
      unlistenTopology = await listenEvent<SpatialGraph>('topology_updated', (graph) => {
        set({ spatialGraph: graph });
      });
    }

    if (!unlistenHitl) {
      unlistenHitl = await listenEvent<HitlAlert>('workflow_paused_hitl', (alert) => {
        set({ hitlAlert: alert, workflowStatus: 'Paused' });
      });
    }

    if (!unlistenSquad) {
      unlistenSquad = await listenEvent<Squad>('squad_status_updated', (squadState) => {
        set({ squad: squadState });
      });
    }
  },

  startMission: async (missionId: string, prompt: string) => {
    set({ workflowStatus: 'Running', hitlAlert: null });
    await invokeCmd<WorkflowView>('compile_mission', { mission_id: missionId, prompt });
    await invokeCmd<WorkflowView>('start_workflow', { mission_id: missionId, workflow_handle: "main_flow" });
  },

  resolveHitl: async (nodeId: string, approved: boolean) => {
    set({ hitlAlert: null, workflowStatus: 'Running' });
    await invokeCmd('resume_workflow', { 
      instance_handle: "current", 
      node_id: nodeId, 
      approved 
    });
  }
}));