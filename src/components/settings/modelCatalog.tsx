import type { ReactNode } from "react";

export interface CatalogModel {
  id: string;
  name: string;
  task: "speech" | "writing";
  purpose: string;
  source: string;
  size: string;
  languages: string;
  runtime: string;
  license: string;
  unavailable?: string;
}

/** Product-facing information; runtime support is deliberately explicit. */
export const MODEL_CATALOG: CatalogModel[] = [
  {
    id: "0.6b",
    name: "Qwen3-ASR 0.6B",
    task: "speech",
    purpose: "A smaller multilingual model for everyday dictation.",
    source: "https://huggingface.co/Qwen/Qwen3-ASR-0.6B-hf",
    size: "1.6 GB Python / 1.0 GB native",
    languages: "30 languages, including English, Chinese and Indian languages",
    runtime: "Python Transformers or experimental native GGUF",
    license: "Apache 2.0",
  },
  {
    id: "1.7b",
    name: "Qwen3-ASR 1.7B",
    task: "speech",
    purpose: "More capacity for difficult recordings; needs more memory.",
    source: "https://huggingface.co/Qwen/Qwen3-ASR-1.7B-hf",
    size: "4.1 GB Python / 2.5 GB native",
    languages: "30 languages",
    runtime: "Python Transformers or experimental native GGUF",
    license: "Apache 2.0",
  },
  {
    id: "phonon-2",
    name: "Phonon-2",
    task: "speech",
    purpose: "Compact English recognition; does not support recognition hotword hints.",
    source: "https://huggingface.co/FermionResearch/Phonon-2",
    size: "164 MB download",
    languages: "English",
    runtime: "Fermion Python runtime",
    license: "See source model card",
  },
  {
    id: "zipformer-20m",
    name: "Zipformer 20M INT8",
    task: "speech",
    purpose: "Small CPU model with streaming partial transcripts.",
    source: "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17",
    size: "44 MB download",
    languages: "English",
    runtime: "sherpa-onnx Python, CPU",
    license: "See source model card",
  },
  {
    id: "qwen3.5-0.8b",
    name: "Qwen3.5 0.8B",
    task: "writing",
    purpose: "Start here for short cleanup and spoken corrections.",
    source: "https://huggingface.co/unsloth/Qwen3.5-0.8B-GGUF",
    size: "508 MB download · runtime memory varies",
    languages: "Multilingual; quality varies by language",
    runtime: "llama.cpp GGUF · Q4_K_M · CPU or GPU",
    license: "Apache 2.0",
  },
  {
    id: "qwen3.5-2b",
    name: "Qwen3.5 2B",
    task: "writing",
    purpose: "More room for paragraph structure and longer writing.",
    source: "https://huggingface.co/unsloth/Qwen3.5-2B-GGUF",
    size: "1.4 GB download · runtime memory varies",
    languages: "Multilingual; quality varies by language",
    runtime: "llama.cpp GGUF · Q4_K_M · CPU or GPU",
    license: "Apache 2.0",
  },
  {
    id: "parakeet-ultra",
    name: "Parakeet Ultra 0.6B",
    task: "speech",
    purpose: "GPU speech recognition for European languages.",
    source: "https://huggingface.co/moondream/parakeet-ultra",
    size: "1.26 GB model download · runtime downloaded separately when needed",
    languages: "25 European languages",
    runtime: "Transformers Parakeet · CPU FP32 or NVIDIA GPU BF16",
    license: "CC BY 4.0",
  },
  {
    id: "indic-transcribe-8bit",
    name: "Indic Transcribe Core 8-bit",
    task: "speech",
    purpose: "An Indic-language speech model with custom inference code.",
    source: "https://huggingface.co/ManiKumarAdapala/indic-transcribe-core-8bit",
    size: "1.57 GB model download · publisher reports about 2.5 GB GPU memory",
    languages: "Indic languages · see source for coverage",
    runtime: "Pinned Transformers custom code + bitsandbytes · NVIDIA CUDA 13 · Linux/Windows x64",
    license: "See source model card",
  },
  {
    id: "omnilingual-300m-int8",
    name: "Omnilingual ASR 300M INT8",
    task: "speech",
    purpose: "Broad language coverage with compact ONNX weights.",
    source: "https://huggingface.co/Edison2ST/sherpa-onnx-omnilingual-asr-1600-languages-ctc-v2",
    size: "247 MB download · about 350 MB on disk",
    languages: "1,600+ languages",
    runtime: "sherpa-onnx Python · ONNX INT8 · CPU; automatic language recognition",
    license: "Apache 2.0",
  },
  {
    id: "qwen3.5-4b",
    name: "Qwen3.5 4B · Q4_K_M",
    task: "writing",
    purpose: "A larger option for complex edits and structured notes.",
    source: "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF",
    size: "2.74 GB download · runtime memory varies",
    languages: "Multilingual",
    runtime: "llama.cpp GGUF · Q4_K_M",
    license: "Apache 2.0",
  },
  {
    id: "minicpm5-2b",
    name: "MiniCPM5 2B",
    task: "writing",
    purpose:
      "Compact writing alternative. Runtime supported; local speed and quality are unmeasured.",
    source: "https://huggingface.co/openbmb/MiniCPM5-2B-GGUF",
    size: "1.56 GB download · runtime memory varies",
    languages: "English and Chinese",
    runtime: "llama.cpp GGUF · Q4_K_M · CPU or GPU",
    license: "Apache 2.0",
  },
  {
    id: "gemma4-e4b",
    name: "Gemma 4 E4B · Q4_K_M",
    task: "writing",
    purpose:
      "Larger writing alternative. Runtime supported; local speed and quality are unmeasured.",
    source: "https://huggingface.co/unsloth/gemma-4-E4B-it-GGUF",
    size: "4.98 GB download · runtime memory varies",
    languages: "See exact checkpoint when available",
    runtime: "llama.cpp GGUF · Q4_K_M · CPU or GPU",
    license: "Model-specific terms · see source",
  },
];

export function CatalogDetails({ model }: { model?: CatalogModel }) {
  if (!model) return null;
  const facts: [string, ReactNode][] = [
    ["Best for", model.purpose],
    ["Size & memory", model.size],
    ["Languages", model.languages],
    ["Runtime", model.runtime],
    ["License", model.license],
    [
      "Source",
      <a
        className="text-accent underline underline-offset-4 break-all"
        href={model.source}
        target="_blank"
        rel="noreferrer"
      >
        {model.source.replace("https://huggingface.co/", "")}
      </a>,
    ],
  ];
  return (
    <details className="mt-3 text-xs text-muted group">
      <summary className="cursor-pointer w-fit text-ink-2 hover:text-accent focus-visible:outline-2 focus-visible:outline-accent rounded-[var(--radius-control)]">
        Model details
      </summary>
      <dl className="grid grid-cols-[6.5rem_minmax(0,1fr)] gap-x-4 gap-y-2 mt-3 leading-relaxed">
        {facts.map(([label, value]) => (
          <div className="contents" key={label}>
            <dt className="text-muted">{label}</dt>
            <dd className="text-ink-2 min-w-0">{value}</dd>
          </div>
        ))}
      </dl>
    </details>
  );
}

export function CatalogUnavailable({ task }: { task: "speech" | "writing" }) {
  return (
    <ul aria-label={`Additional ${task} models`} className="divide-y divide-line-soft">
      {MODEL_CATALOG.filter((model) => model.task === task && model.unavailable).map((model) => (
        <li key={model.id} className="py-4 first:pt-2 last:pb-0">
          <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
            <p className="text-sm font-medium text-ink">{model.name}</p>
            <span className="text-2xs text-muted">Runtime unavailable</span>
          </div>
          <p className="text-xs text-muted leading-relaxed mt-1">{model.purpose}</p>
          <p className="text-2xs text-muted mt-1">
            {model.languages} · {model.size}
          </p>
          <p className="text-xs text-warning leading-relaxed mt-2">{model.unavailable}</p>
          <CatalogDetails model={model} />
        </li>
      ))}
    </ul>
  );
}
