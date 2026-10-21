{{/* Chart name. */}}
{{- define "greybeard.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/* Fully qualified app name. */}}
{{- define "greybeard.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{- define "greybeard.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "greybeard.labels" -}}
helm.sh/chart: {{ include "greybeard.chart" . }}
{{ include "greybeard.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{- define "greybeard.selectorLabels" -}}
app.kubernetes.io/name: {{ include "greybeard.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "greybeard.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "greybeard.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/* Name of the Secret the pod reads, whatever the secrets mode. */}}
{{- define "greybeard.secretName" -}}
{{- default (include "greybeard.fullname" .) .Values.secrets.existingSecret }}
{{- end }}

{{/* True when the chart itself renders a Secret. */}}
{{- define "greybeard.createSecret" -}}
{{- if and (not .Values.secrets.existingSecret) (not .Values.secrets.externalSecret.enabled) }}true{{ end }}
{{- end }}

{{/* Input validation — fail fast at render time with a clear message. */}}
{{- define "greybeard.validate" -}}
{{- $r := int .Values.replicaCount }}
{{- if not (or (eq $r 0) (eq $r 1)) }}
{{- fail (printf "replicaCount must be 0 or 1 (got %v): greybeard is single-writer — the PR marker comment is its state store" .Values.replicaCount) }}
{{- end }}
{{- $p := .Values.llm.provider }}
{{- if not (has $p (list "anthropic" "bedrock")) }}
{{- fail (printf "llm.provider must be 'anthropic' or 'bedrock' (got %q)" $p) }}
{{- end }}
{{- if eq $p "bedrock" }}
{{- if not .Values.llm.lensModel }}
{{- fail "llm.lensModel is required when llm.provider=bedrock (e.g. a Bedrock inference profile ID)" }}
{{- end }}
{{- if not .Values.llm.awsRegion }}
{{- fail "llm.awsRegion is required when llm.provider=bedrock" }}
{{- end }}
{{- end }}
{{- if not .Values.github.botLogin }}
{{- fail "github.botLogin is required, e.g. 'my-reviewer[bot]'" }}
{{- end }}
{{- if and .Values.secrets.existingSecret .Values.secrets.externalSecret.enabled }}
{{- fail "set only one of secrets.existingSecret or secrets.externalSecret.enabled" }}
{{- end }}
{{- if .Values.secrets.externalSecret.enabled }}
{{- $es := .Values.secrets.externalSecret }}
{{- if not $es.secretStoreRef.name }}
{{- fail "secrets.externalSecret.secretStoreRef.name is required" }}
{{- end }}
{{- if or (not $es.remoteRefs.webhookSecret) (not $es.remoteRefs.privateKey) }}
{{- fail "secrets.externalSecret.remoteRefs.webhookSecret and .privateKey are required" }}
{{- end }}
{{- if and (eq $p "anthropic") (not $es.remoteRefs.anthropicApiKey) }}
{{- fail "secrets.externalSecret.remoteRefs.anthropicApiKey is required when llm.provider=anthropic" }}
{{- end }}
{{- if and (not .Values.github.appId) (not $es.remoteRefs.appId) }}
{{- fail "set github.appId or secrets.externalSecret.remoteRefs.appId" }}
{{- end }}
{{- else if include "greybeard.createSecret" . }}
{{- $c := .Values.secrets.create }}
{{- if or (not $c.webhookSecret) (not $c.privateKey) }}
{{- fail "no secret source: set secrets.existingSecret, enable secrets.externalSecret, or provide secrets.create.webhookSecret and .privateKey" }}
{{- end }}
{{- if and (eq $p "anthropic") (not $c.anthropicApiKey) }}
{{- fail "secrets.create.anthropicApiKey is required when llm.provider=anthropic" }}
{{- end }}
{{- if and (not .Values.github.appId) (not $c.appId) }}
{{- fail "set github.appId or secrets.create.appId" }}
{{- end }}
{{- end }}
{{- end }}
