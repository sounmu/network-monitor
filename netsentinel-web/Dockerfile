FROM node:20-alpine AS base

# 패키지 빌드 및 설치를 위한 builder 단계
FROM base AS builder
WORKDIR /app

# npm 의존성 복사 및 설치
COPY package.json package-lock.json* ./
RUN npm ci

# 빌드 인자 선언 및 검증
ARG NEXT_PUBLIC_API_URL
RUN if [ -z "$NEXT_PUBLIC_API_URL" ]; then \
      echo "ERROR: NEXT_PUBLIC_API_URL build arg is empty!" && exit 1; \
    fi
ENV NEXT_PUBLIC_API_URL=$NEXT_PUBLIC_API_URL
ENV NEXT_TELEMETRY_DISABLED=1

# 소스 복사 및 프로젝트 빌드
COPY . .
RUN echo "Building with NEXT_PUBLIC_API_URL=$NEXT_PUBLIC_API_URL" && npm run build

# 빌드 산출물을 실행할 runner 단계 (최소한의 파일만 포함하는 standalone 모드)
FROM base AS runner
WORKDIR /app

ENV NODE_ENV=production
ENV NEXT_TELEMETRY_DISABLED=1

# 보안을 위해 권한이 제한된 유저 생성
RUN addgroup --system --gid 1001 nodejs
RUN adduser --system --uid 1001 nextjs

# .next 폴더 권한 할당
RUN mkdir .next
RUN chown nextjs:nodejs .next

# standalone 폴더의 파일들을 runner로 복사 (서버 실행에 필요한 필수 파일들)
COPY --from=builder --chown=nextjs:nodejs /app/.next/standalone ./

# tĩnh 자원(정적 파일) 복사
COPY --from=builder --chown=nextjs:nodejs /app/public ./public
COPY --from=builder --chown=nextjs:nodejs /app/.next/static ./.next/static

# root 대신 nextjs 유저로 실행
USER nextjs

ENV PORT=3000
ENV HOSTNAME="0.0.0.0"

# standalone 모드로 생성된 node js 서버 진입점 실행
CMD ["node", "server.js"]
