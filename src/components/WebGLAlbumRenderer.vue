<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import * as THREE from 'three';
import { RoundedBoxGeometry } from 'three/examples/jsm/geometries/RoundedBoxGeometry.js';

const props = defineProps<{
  coverUrl: string | null;
  colors: [string, string];
  rarityColor: string;
  beatScale: number;
  isPlaying: boolean;
  tiltX: number;
  tiltY: number;
  offsetX: number;
  offsetY: number;
  dragging: boolean;
}>();

const hostRef = ref<HTMLDivElement | null>(null);
const failedRef = ref(false);

const fallbackStyle = computed(() => {
  if (props.coverUrl) {
    return {
      backgroundImage: `url(${props.coverUrl})`,
      backgroundSize: 'cover',
      backgroundPosition: 'center',
    };
  }
  return {
    background: `linear-gradient(135deg, ${props.colors[0]}, ${props.colors[1]})`,
  };
});

const SHEEN_VERTEX_SHADER = `
  varying vec2 vUv;

  void main() {
    vUv = uv;
    gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
  }
`;

const SHEEN_FRAGMENT_SHADER = `
  uniform float uTime;
  uniform float uBeat;
  uniform float uPlaying;
  uniform vec2 uTilt;
  uniform vec3 uRarityColor;
  uniform sampler2D uMask;

  varying vec2 vUv;

  float stripePulse(float x) {
    return smoothstep(0.7, 1.0, sin(x));
  }

  void main() {
    float mask = texture2D(uMask, vUv).r;
    vec2 centered = vUv - 0.5;
    vec2 dir = normalize(vec2(0.88 + uTilt.y * 0.85, -0.42 + uTilt.x * 0.85));
    vec2 stripeDir = vec2(-dir.y, dir.x);

    float lightOffset = clamp(dot(uTilt, vec2(0.16, -0.14)), -0.16, 0.16);
    float band = abs(dot(centered, dir) - lightOffset);
    float highlight = smoothstep(0.24, 0.02, band);

    float flow = dot(centered, stripeDir) * 84.0 + uTime * (0.55 + uPlaying * 2.8);
    float stripes = stripePulse(flow) * 0.74 + stripePulse(flow * 0.61 + 1.7) * 0.42;
    float spark = smoothstep(0.82, 1.0, sin(flow * 0.45 - uTime * 1.2));

    float intensity = highlight * (0.08 + uBeat * 0.18 + stripes * 0.46 + spark * 0.18) * (0.25 + uPlaying * 0.75);
    vec3 tinted = mix(vec3(1.0), uRarityColor, 0.78);
    vec3 color = tinted * intensity + vec3(1.0) * spark * highlight * 0.12;

    gl_FragColor = vec4(color, mask * intensity);
  }
`;

let renderer: THREE.WebGLRenderer | null = null;
let scene: THREE.Scene | null = null;
let camera: THREE.PerspectiveCamera | null = null;
let resizeObserver: ResizeObserver | null = null;
let animationFrame = 0;
let disposed = false;
let textureLoadVersion = 0;
let lastFrameTime = 0;

let coverGroup: THREE.Group | null = null;
let coverBodyMesh: THREE.Mesh | null = null;
let coverArtMesh: THREE.Mesh | null = null;
let sheenMesh: THREE.Mesh | null = null;

let coverBodyMaterial: THREE.MeshStandardMaterial | null = null;
let coverArtMaterial: THREE.MeshPhysicalMaterial | null = null;
let sheenMaterial: THREE.ShaderMaterial | null = null;

let colorWashLight: THREE.PointLight | null = null;
let rimLight: THREE.PointLight | null = null;

let coverTexture: THREE.Texture | null = null;
let coverMaskTexture: THREE.Texture | null = null;

const motionState = {
  coverRotX: 0,
  coverRotY: 0,
  coverPosX: 0,
  coverPosY: 0,
  coverScale: 1,
};

function disposeTexture(texture: THREE.Texture | null) {
  texture?.dispose();
}

function createGradientTexture(colors: [string, string]) {
  const canvas = document.createElement('canvas');
  canvas.width = 1024;
  canvas.height = 1024;
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;

  const gradient = ctx.createLinearGradient(0, 0, canvas.width, canvas.height);
  gradient.addColorStop(0, colors[0]);
  gradient.addColorStop(1, colors[1]);
  ctx.fillStyle = gradient;
  ctx.fillRect(0, 0, canvas.width, canvas.height);

  const glow = ctx.createRadialGradient(canvas.width * 0.34, canvas.height * 0.26, 0, canvas.width * 0.34, canvas.height * 0.26, canvas.width * 0.5);
  glow.addColorStop(0, 'rgba(255,255,255,0.24)');
  glow.addColorStop(1, 'rgba(255,255,255,0)');
  ctx.fillStyle = glow;
  ctx.fillRect(0, 0, canvas.width, canvas.height);

  ctx.fillStyle = 'rgba(255,255,255,0.06)';
  for (let row = 0; row < 16; row += 1) {
    const y = (row / 16) * canvas.height;
    ctx.fillRect(0, y, canvas.width, canvas.height / 140);
  }

  const vignette = ctx.createRadialGradient(canvas.width / 2, canvas.height / 2, canvas.width * 0.18, canvas.width / 2, canvas.height / 2, canvas.width * 0.72);
  vignette.addColorStop(0, 'rgba(0,0,0,0)');
  vignette.addColorStop(1, 'rgba(0,0,0,0.42)');
  ctx.fillStyle = vignette;
  ctx.fillRect(0, 0, canvas.width, canvas.height);

  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  return texture;
}

function createRoundedMaskTexture() {
  const canvas = document.createElement('canvas');
  canvas.width = 1024;
  canvas.height = 1024;
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;

  const radius = 96;
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  ctx.beginPath();
  ctx.moveTo(radius, 0);
  ctx.lineTo(canvas.width - radius, 0);
  ctx.quadraticCurveTo(canvas.width, 0, canvas.width, radius);
  ctx.lineTo(canvas.width, canvas.height - radius);
  ctx.quadraticCurveTo(canvas.width, canvas.height, canvas.width - radius, canvas.height);
  ctx.lineTo(radius, canvas.height);
  ctx.quadraticCurveTo(0, canvas.height, 0, canvas.height - radius);
  ctx.lineTo(0, radius);
  ctx.quadraticCurveTo(0, 0, radius, 0);
  ctx.closePath();
  ctx.fillStyle = '#fff';
  ctx.fill();

  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.NoColorSpace;
  return texture;
}

function setRendererSize() {
  const host = hostRef.value;
  if (!host || !renderer || !camera) return;
  const width = host.clientWidth;
  const height = host.clientHeight;
  if (!width || !height) return;

  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 1.6));
  renderer.setSize(width, height, false);
  camera.aspect = width / height;
  camera.updateProjectionMatrix();
}

function applyScenePalette() {
  const primary = new THREE.Color(props.colors[0]);
  const secondary = new THREE.Color(props.colors[1]);
  const body = primary.clone().lerp(secondary, 0.42).multiplyScalar(0.28);

  if (coverBodyMaterial) {
    coverBodyMaterial.color.copy(body);
    coverBodyMaterial.emissive.copy(body.clone().multiplyScalar(0.22));
  }
  if (colorWashLight) {
    colorWashLight.color.copy(secondary);
  }
  if (rimLight) {
    rimLight.color.copy(primary);
  }
  if (sheenMaterial) {
    sheenMaterial.uniforms.uRarityColor.value.set(props.rarityColor);
  }
}

async function loadCoverTexture(url: string) {
  const loader = new THREE.TextureLoader();
  return await new Promise<THREE.Texture>((resolve, reject) => {
    loader.load(url, resolve, undefined, reject);
  });
}

async function updateTextures() {
  const version = ++textureLoadVersion;
  let nextCoverTexture: THREE.Texture | null = null;
  try {
    nextCoverTexture = props.coverUrl ? await loadCoverTexture(props.coverUrl) : null;
  } catch {
    nextCoverTexture = null;
  }

  if (disposed || version !== textureLoadVersion) {
    disposeTexture(nextCoverTexture);
    return;
  }

  if (!nextCoverTexture) {
    nextCoverTexture = createGradientTexture(props.colors);
  }

  if (nextCoverTexture) {
    nextCoverTexture.colorSpace = THREE.SRGBColorSpace;
    nextCoverTexture.anisotropy = renderer?.capabilities.getMaxAnisotropy() ?? 1;
  }

  disposeTexture(coverTexture);
  coverTexture = nextCoverTexture;

  if (coverArtMaterial) {
    coverArtMaterial.map = coverTexture;
    coverArtMaterial.needsUpdate = true;
  }
}

function buildScene() {
  scene = new THREE.Scene();

  camera = new THREE.PerspectiveCamera(26, 1, 0.1, 100);
  camera.position.set(0, 0.08, 5.0);

  renderer = new THREE.WebGLRenderer({
    antialias: true,
    alpha: true,
    powerPreference: 'high-performance',
  });
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.08;

  const host = hostRef.value;
  if (!host) return;
  host.appendChild(renderer.domElement);

  const ambient = new THREE.HemisphereLight(0xffffff, 0x10131a, 1.6);
  scene.add(ambient);

  const keyLight = new THREE.DirectionalLight(0xffffff, 1.9);
  keyLight.position.set(1.8, 2.2, 3.6);
  scene.add(keyLight);

  colorWashLight = new THREE.PointLight(new THREE.Color(props.colors[1]), 1.05, 12, 2);
  colorWashLight.position.set(-1.5, -0.1, 2.8);
  scene.add(colorWashLight);

  rimLight = new THREE.PointLight(new THREE.Color(props.colors[0]), 0.72, 9, 2);
  rimLight.position.set(1.7, 0.35, 1.9);
  scene.add(rimLight);

  coverGroup = new THREE.Group();
  coverGroup.position.z = 0.14;
  scene.add(coverGroup);

  coverBodyMaterial = new THREE.MeshStandardMaterial({
    color: '#161922',
    roughness: 0.56,
    metalness: 0.12,
    emissive: new THREE.Color('#05070b'),
    emissiveIntensity: 0.65,
  });

  const coverBodyGeometry = new RoundedBoxGeometry(1.72, 1.72, 0.11, 5, 0.08);
  coverBodyMesh = new THREE.Mesh(coverBodyGeometry, coverBodyMaterial);
  coverGroup.add(coverBodyMesh);

  coverMaskTexture = createRoundedMaskTexture();
  coverArtMaterial = new THREE.MeshPhysicalMaterial({
    color: '#ffffff',
    roughness: 0.3,
    metalness: 0.02,
    clearcoat: 1,
    clearcoatRoughness: 0.16,
    alphaMap: coverMaskTexture ?? undefined,
    transparent: true,
    side: THREE.DoubleSide,
  });
  const coverArtGeometry = new THREE.PlaneGeometry(1.68, 1.68);
  coverArtMesh = new THREE.Mesh(coverArtGeometry, coverArtMaterial);
  coverArtMesh.position.z = 0.061;
  coverGroup.add(coverArtMesh);

  sheenMaterial = new THREE.ShaderMaterial({
    uniforms: {
      uTime: { value: 0 },
      uBeat: { value: 0 },
      uPlaying: { value: 0 },
      uTilt: { value: new THREE.Vector2(0, 0) },
      uRarityColor: { value: new THREE.Color(props.rarityColor) },
      uMask: { value: coverMaskTexture },
    },
    vertexShader: SHEEN_VERTEX_SHADER,
    fragmentShader: SHEEN_FRAGMENT_SHADER,
    transparent: true,
    depthWrite: false,
    depthTest: true,
    blending: THREE.AdditiveBlending,
    side: THREE.DoubleSide,
  });
  const sheenGeometry = new THREE.PlaneGeometry(1.7, 1.7);
  sheenMesh = new THREE.Mesh(sheenGeometry, sheenMaterial);
  sheenMesh.position.z = 0.064;
  coverGroup.add(sheenMesh);

  applyScenePalette();
  setRendererSize();
}

function animate(now: number) {
  if (disposed || !renderer || !scene || !camera || !coverGroup) {
    return;
  }

  if (!lastFrameTime) lastFrameTime = now;
  const delta = Math.min((now - lastFrameTime) / 1000, 0.05);
  lastFrameTime = now;
  const beat = Math.max(0, Math.min(1.4, (props.beatScale - 1) / 0.1));

  const follow = 1 - Math.exp(-delta * 8);
  const targetRotX = THREE.MathUtils.degToRad(props.tiltX * 0.88);
  const targetRotY = THREE.MathUtils.degToRad(props.tiltY * 0.88);
  const targetPosX = props.offsetX / 280;
  const targetPosY = -props.offsetY / 280;
  const targetScale = 1 + (props.dragging ? 0.035 : 0) + (props.beatScale - 1) * 0.55;

  motionState.coverRotX = THREE.MathUtils.lerp(motionState.coverRotX, targetRotX, follow);
  motionState.coverRotY = THREE.MathUtils.lerp(motionState.coverRotY, targetRotY, follow);
  motionState.coverPosX = THREE.MathUtils.lerp(motionState.coverPosX, targetPosX, follow);
  motionState.coverPosY = THREE.MathUtils.lerp(motionState.coverPosY, targetPosY, follow);
  motionState.coverScale = THREE.MathUtils.lerp(motionState.coverScale, targetScale, follow);

  coverGroup.rotation.x = motionState.coverRotX;
  coverGroup.rotation.y = motionState.coverRotY;
  coverGroup.position.x = motionState.coverPosX;
  coverGroup.position.y = motionState.coverPosY;
  coverGroup.scale.setScalar(motionState.coverScale);

  if (colorWashLight) {
    colorWashLight.intensity = 1.0 + beat * 0.32;
  }

  if (sheenMaterial) {
    sheenMaterial.uniforms.uTime.value = now * 0.001;
    sheenMaterial.uniforms.uBeat.value = beat;
    sheenMaterial.uniforms.uPlaying.value = props.isPlaying ? 1 : 0;
    sheenMaterial.uniforms.uTilt.value.set(props.tiltX / 18, props.tiltY / 18);
    sheenMaterial.uniforms.uRarityColor.value.set(props.rarityColor);
  }

  renderer.render(scene, camera);
  animationFrame = requestAnimationFrame(animate);
}

function cleanup() {
  disposed = true;
  cancelAnimationFrame(animationFrame);
  lastFrameTime = 0;
  resizeObserver?.disconnect();
  resizeObserver = null;

  if (renderer) {
    renderer.dispose();
    renderer.domElement.remove();
    renderer = null;
  }

  scene?.traverse((node: THREE.Object3D) => {
    if (node instanceof THREE.Mesh) {
      node.geometry.dispose();
      if (Array.isArray(node.material)) {
        node.material.forEach((material: THREE.Material) => material.dispose());
      } else {
        node.material.dispose();
      }
    }
  });

  scene = null;
  camera = null;
  coverGroup = null;
  coverBodyMesh = null;
  coverArtMesh = null;
  sheenMesh = null;
  coverBodyMaterial = null;
  coverArtMaterial = null;
  sheenMaterial = null;
  colorWashLight = null;
  rimLight = null;

  disposeTexture(coverTexture);
  disposeTexture(coverMaskTexture);
  coverTexture = null;
  coverMaskTexture = null;
}

onMounted(async () => {
  try {
    buildScene();
    resizeObserver = new ResizeObserver(() => setRendererSize());
    if (hostRef.value) resizeObserver.observe(hostRef.value);
    await updateTextures();
    animationFrame = requestAnimationFrame(animate);
  } catch {
    failedRef.value = true;
    cleanup();
  }
});

watch(() => props.coverUrl, () => {
  if (scene) void updateTextures();
});

watch(() => [props.colors[0], props.colors[1], props.rarityColor], () => {
  applyScenePalette();
  if (scene) void updateTextures();
});

onUnmounted(() => cleanup());
</script>

<template>
  <div ref="hostRef" class="webgl-album-renderer">
    <div v-if="failedRef" class="webgl-album-fallback" :style="fallbackStyle" />
  </div>
</template>

<style scoped>
.webgl-album-renderer {
  position: absolute;
  inset: 0;
  z-index: 2;
  overflow: visible;
}

.webgl-album-renderer :deep(canvas) {
  width: 100%;
  height: 100%;
  display: block;
}

.webgl-album-fallback {
  position: absolute;
  inset: 8%;
  border-radius: 14px;
  box-shadow: 0 24px 60px rgba(0, 0, 0, 0.35);
}
</style>
