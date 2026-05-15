FROM node:20-alpine AS build

WORKDIR /app
COPY package.json tsconfig.json ./
RUN npm install
COPY src/ ./src/
RUN npm run build

FROM node:20-alpine AS runtime

WORKDIR /app
COPY --from=build /app/dist ./dist
COPY --from=build /app/node_modules ./node_modules
COPY config.yaml.template ./config.yaml.template

EXPOSE 3001

ENV CONFIG_PATH=/etc/pstep-gateway/config.yaml

CMD ["node", "dist/server.js"]