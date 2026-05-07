{{- define "jattg.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "jattg.namespace" -}}
{{- default .Release.Namespace .Values.namespace.name -}}
{{- end -}}

{{- define "jattg.labels" -}}
app.kubernetes.io/name: {{ include "jattg.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ .Chart.Name }}-{{ .Chart.Version | replace "+" "_" }}
{{- end -}}

{{- define "jattg.image" -}}
{{- $root := index . "root" -}}
{{- $image := index . "image" -}}
{{- $tag := default $root.Values.global.imageTag $image.tag -}}
{{- printf "%s:%s" $image.repository $tag -}}
{{- end -}}
