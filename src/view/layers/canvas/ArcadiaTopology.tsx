import { useRef, useMemo, useEffect } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { useRaiseStore } from '../../../store/useRaiseStore';
import { LayerType } from '../../../types/bindings';

// Palette de couleurs cyberpunk/industrielle selon la couche Arcadia
const LAYER_COLORS: Record<LayerType, string> = {
  [LayerType.OA]: '#ef4444', // Rouge (Operational)
  [LayerType.SA]: '#f59e0b', // Ambre (System)
  [LayerType.LA]: '#10b981', // Émeraude (Logical)
  [LayerType.PA]: '#3b82f6', // Bleu (Physical)
  [LayerType.Chaos]: '#8b5cf6', // Violet (IA/Non-structuré)
};

export default function ArcadiaTopology() {
  const spatialGraph = useRaiseStore((state) => state.spatialGraph);
  
  const meshRef = useRef<THREE.InstancedMesh>(null);
  const dummy = useMemo(() => new THREE.Object3D(), []);
  const color = useMemo(() => new THREE.Color(), []);

  // 1. Génération ultra-rapide des arêtes (Links) en une seule géométrie
  const linesGeometry = useMemo(() => {
    if (!spatialGraph) return null;
    
    const positions: number[] = [];
    // Map pour un accès O(1) aux positions des nœuds
    const nodeMap = new Map(spatialGraph.nodes.map(n => [n.id, n.position]));

    spatialGraph.links.forEach(link => {
      const sourcePos = nodeMap.get(link.source);
      const targetPos = nodeMap.get(link.target);
      if (sourcePos && targetPos) {
        positions.push(...sourcePos, ...targetPos);
      }
    });

    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute('position', new THREE.Float32BufferAttribute(positions, 3));
    return geometry;
  }, [spatialGraph]);

  // 2. Initialisation des positions et couleurs de l'InstancedMesh
  useEffect(() => {
    if (!spatialGraph || !meshRef.current) return;

    spatialGraph.nodes.forEach((node, i) => {
      // Position
      dummy.position.set(node.position[0], node.position[1], node.position[2]);
      dummy.updateMatrix();
      meshRef.current!.setMatrixAt(i, dummy.matrix);

      // Couleur
      color.set(LAYER_COLORS[node.layer] || '#ffffff');
      meshRef.current!.setColorAt(i, color);
    });

    meshRef.current.instanceMatrix.needsUpdate = true;
    if (meshRef.current.instanceColor) {
      meshRef.current.instanceColor.needsUpdate = true;
    }
  }, [spatialGraph, dummy, color]);

  // 3. Boucle de rendu : Animation des vibrations (Stabilité)
  useFrame((state) => {
    if (!spatialGraph || !meshRef.current) return;

    let needsUpdate = false;
    const time = state.clock.elapsedTime;

    spatialGraph.nodes.forEach((node, i) => {
      // Si la stabilité n'est pas parfaite, on fait vibrer le nœud
      if (node.stability < 1.0) {
        needsUpdate = true;
        const intensity = (1.0 - node.stability) * 0.4; // Force de la vibration
        
        // Jitter pseudo-aléatoire basé sur le temps et l'index
        const shakeX = Math.sin(time * 30 + i) * intensity;
        const shakeY = Math.cos(time * 25 + i) * intensity;
        const shakeZ = Math.sin(time * 35 + i) * intensity;

        dummy.position.set(
          node.position[0] + shakeX,
          node.position[1] + shakeY,
          node.position[2] + shakeZ
        );
        dummy.updateMatrix();
        meshRef.current!.setMatrixAt(i, dummy.matrix);
      }
    });

    if (needsUpdate) {
      meshRef.current.instanceMatrix.needsUpdate = true;
    }
  });

  if (!spatialGraph) return null;

  return (
    <group>
      {/* Rendu des Arêtes */}
      {linesGeometry && (
        <lineSegments geometry={linesGeometry}>
          <lineBasicMaterial color="#475569" transparent opacity={0.6} />
        </lineSegments>
      )}

      {/* Rendu des Nœuds (Instanciation massive) */}
      <instancedMesh
        ref={meshRef}
        args={[undefined, undefined, spatialGraph.meta.node_count]}
      >
        <sphereGeometry args={[0.5, 16, 16]} />
        <meshStandardMaterial roughness={0.3} metalness={0.8} />
      </instancedMesh>
    </group>
  );
}