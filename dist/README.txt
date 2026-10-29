Drifting VOID {VERSION}

설치
  1. 압축을 모두 풉니다 (압축 파일 안에서 바로 실행하지 마세요).
  2. Setup.cmd를 더블 클릭합니다.
     "Windows의 PC 보호" 창이 뜨면 [추가 정보] → [실행]을 누릅니다.
     (아직 코드 서명이 없어서 나오는 창입니다.)
  3. 영상·소리에 쓰는 ffmpeg가 없으면 GitHub에서 내려받을지 묻습니다 (약 80 MB).
     관리자 권한은 필요 없습니다. 설치 위치: %LOCALAPPDATA%\Programs\DriftingVOID

사용 설명서
  manual\index.html 을 브라우저로 엽니다. 설치 후에는 시작 메뉴에도 있습니다.

제거
  설정 > 앱 > Drifting VOID > 제거
  .void 프로젝트 파일은 지우지 않습니다.

문제가 생기면
  %APPDATA%\DriftingVOID\crash.log 를 함께 보내 주세요.

ffmpeg
  FFmpeg 프로젝트(https://ffmpeg.org)의 별도 프로그램으로, GPL 라이선스입니다.
  이 패키지에는 들어 있지 않으며, 설치 과정에서 GitHub의 BtbN/FFmpeg-Builds
  공개 빌드를 받아 SHA-256을 확인한 뒤 설치 폴더의 ffmpeg 폴더에만 둡니다.
  라이선스 전문과 출처는 그 폴더의 LICENSE.txt, SOURCE.txt에 있습니다.
