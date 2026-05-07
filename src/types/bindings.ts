/**
 * BINDINGS TAURI / RUST -> TYPESCRIPT
 * Ce fichier est la source de vérité (Single Source of Truth) pour l'UI.
 */

// ============================================================================
// 1. WORKFLOW ENGINE & HITL (Mappé sur workflow_commands.rs)
// ============================================================================

export type ExecutionStatus = 'Idle' | 'Running' | 'Paused' | 'Completed' | 'Failed';

export interface WorkflowView {
  handle: string;
  status: ExecutionStatus;
  current_nodes: string[];
  logs: string[];
}

// Représentation de l'événement HITL émis par Rust
export interface HitlAlert {
  nodeId: string;
  message: string;
  riskLevel: 'Red' | 'Orange' | 'Blue';
  context?: Record<string, unknown>;
}

// ============================================================================
// 2. IA MULTI-AGENTS (Mappé sur squad.rs)
// ============================================================================

// Correspond au #[serde(rename_all = "lowercase")] dans Rust
export type SquadStatus = 'active' | 'training' | 'suspended' | 'retired';

export interface Squad {
  _id?: string;
  handle: string;
  name: string; // I18nString côté Rust, on assume une string traduite au runtime UI
  description?: string;
  team_id?: string;
  lead_agent_id: string;
  agents: string[]; // Tableau d'UniqueId (UUID stringifiés)
  capabilities: string[];
  status: SquadStatus;
}

// ============================================================================
// 3. SPATIAL ENGINE / 3D GRAPH (Mappé sur spatial_engine/mod.rs)
// ============================================================================

export enum LayerType {
  OA = 0,    // Operational Analysis
  SA = 1,    // System Analysis
  LA = 2,    // Logical Architecture
  PA = 3,    // Physical Architecture
  Chaos = 4, // Zone IA / Non-structurée
}

export interface SpatialNode {
  id: string;
  label: string;
  position: [number, number, number]; // [x, y, z] parfait pour Three.js
  layer: LayerType;
  weight: number;
  stability: number; // 0.0 (Vibration critique) -> 1.0 (Stable)
}

export interface SpatialLink {
  source: string;
  target: string;
  strength: number;
}

export interface GraphMeta {
  node_count: number;
  layer_distribution: [number, number, number, number, number];
}

export interface SpatialGraph {
  nodes: SpatialNode[];
  links: SpatialLink[];
  meta: GraphMeta;
}

// ============================================================================
// 4. API COMMANDES RUST (Payloads)
// ============================================================================

// Les arguments envoyés depuis React vers les commandes Tauri
export interface StartWorkflowPayload {
  mission_id: string;
  workflow_handle: string;
}

export interface ResumeWorkflowPayload {
  instance_handle: string;
  node_id: string;
  approved: boolean;
}