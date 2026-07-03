{{- define "synthetics.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "synthetics.fullname" -}}
{{- printf "%s-%s" .Release.Name (include "synthetics.name" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "synthetics.labels" -}}
app.kubernetes.io/name: {{ include "synthetics.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "synthetics.dbCluster" -}}
{{- printf "%s-db" (include "synthetics.fullname" .) -}}
{{- end -}}

{{- define "synthetics.cpService" -}}
{{- printf "%s-control-plane" (include "synthetics.fullname" .) -}}
{{- end -}}

{{- define "synthetics.mockService" -}}
{{- printf "%s-mock-datadog" .Release.Name -}}
{{- end -}}

{{- define "synthetics.dogstatsdTarget" -}}
{{- if .Values.mockDatadog.enabled -}}
{{- printf "%s:8125" (include "synthetics.mockService" .) -}}
{{- else -}}
{{- .Values.worker.dogstatsd -}}
{{- end -}}
{{- end -}}
