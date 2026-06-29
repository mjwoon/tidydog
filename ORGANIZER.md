# TidyDog Organizer Rules
# Version: 1.0
#
# C-E 공유 계약: 의미 기반 proposed_dest 규칙 파일.
#
# 형식: <조건들> -> <목적지 경로>
# ─────────────────────────────────────────────────────────────
# 조건 타입 (공백 = AND, 콤마 = OR):
#   tag:<값>       topic_tags에 해당 값이 포함될 때 (대소문자 무시)
#   lang:<값>      language가 해당 BCP-47 코드와 같을 때
#   ext:<값>       파일 확장자 (점 없이, 예: pdf, txt)
#   name:<부분문자열> 파일명에 해당 문자열이 포함될 때 (대소문자 무시)
#
# 규칙 평가: 위에서 아래 순서. 첫 번째로 일치한 규칙만 적용.
# 콤마는 해당 조건 내 OR (tag:세금,tax = tag:세금 OR tag:tax).
# 같은 줄 조건들은 AND (tag:계약 lang:ko = 두 조건 모두 충족).
# ─────────────────────────────────────────────────────────────

# 세금·재무
tag:세금,tax -> 문서/행정/세금

# 의료·건강
tag:의료,건강,health -> 문서/의료

# 영수증
tag:영수증,receipt,invoice -> 문서/영수증
name:영수증,receipt -> 문서/영수증

# 계약서
tag:계약,contract -> 문서/계약

# 여행
tag:여행,travel,여권,passport -> 문서/여행

# 교육·학업
tag:교육,학습,study,school -> 문서/교육

# 업무·직장
tag:업무,work,직장,회사 -> 문서/업무

# PDF (한국어)
ext:pdf lang:ko -> 문서/PDF

# 텍스트 파일 (한국어)
ext:txt,md lang:ko -> 문서/텍스트

# 이미지
ext:jpg,jpeg,png,heic -> 사진

# 한국어 기타
lang:ko -> 문서/기타

# 영어 문서
ext:pdf lang:en -> Documents/PDF
ext:txt,md lang:en -> Documents/Text
lang:en -> Documents/Misc
