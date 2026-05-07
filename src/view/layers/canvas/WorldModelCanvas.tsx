import { Canvas } from '@react-three/fiber';
import { OrbitControls, Stars } from '@react-three/drei';
import ArcadiaTopology from './ArcadiaTopology';

export default function WorldModelCanvas() {
  return (
    <div className="w-full h-full">
      <Canvas camera={{ position: [20, 15, 20], fov: 50 }}>
        {/* Lumières */}
        <ambientLight intensity={0.4} />
        <directionalLight position={[10, 20, 10]} intensity={1.5} color="#e2e8f0" />
        <pointLight position={[-10, -10, -10]} intensity={0.5} color="#38bdf8" />

        {/* Environnement (Fond galactique discret) */}
        <Stars radius={100} depth={50} count={3000} factor={4} saturation={0} fade speed={1} />

        {/* Contrôles de la caméra (Souris/Touch) */}
        <OrbitControls 
          enableDamping 
          dampingFactor={0.05} 
          minDistance={5} 
          maxDistance={100} 
        />

        {/* La Topologie Raise */}
        <ArcadiaTopology />
      </Canvas>
    </div>
  );
}