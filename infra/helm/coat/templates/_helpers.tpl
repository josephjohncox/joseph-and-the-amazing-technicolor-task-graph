{{- define "coat.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "coat.namespace" -}}
{{- default .Release.Namespace .Values.namespace.name -}}
{{- end -}}

{{- define "coat.labels" -}}
app.kubernetes.io/name: {{ include "coat.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ .Chart.Name }}-{{ .Chart.Version | replace "+" "_" }}
{{- end -}}

{{- define "coat.image" -}}
{{- $root := index . "root" -}}
{{- $image := index . "image" -}}
{{- $tag := default $root.Values.global.imageTag $image.tag -}}
{{- printf "%s:%s" $image.repository $tag -}}
{{- end -}}
